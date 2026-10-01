use crate::{
    engine::{Edit, History, Segmentation, Settings, Stroke},
    interaction::View,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub(crate) struct Session {
    pub version: u32,
    pub photos: Vec<SavedPhoto>,
    #[serde(default)]
    pub current: usize,
    #[serde(default)]
    pub compare: bool,
    #[serde(default = "center_split")]
    pub split: f32,
    #[serde(default)]
    pub clean_shutdown: bool,
}

fn center_split() -> f32 {
    0.5
}
fn center_crop() -> [f32; 2] {
    [0.5; 2]
}
fn selected() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedPhoto {
    pub path: Option<PathBuf>,
    pub edit: Edit,
    pub rating: u8,
    #[serde(default)]
    pub segmentation: Option<Segmentation>,
    #[serde(default)]
    pub view: View,
    #[serde(default = "center_crop")]
    pub crop_center: [f32; 2],
    #[serde(default, with = "compact_history")]
    pub history: History,
    #[serde(default = "selected")]
    pub selected: bool,
}

pub(crate) fn read(path: &Path) -> Result<Session> {
    let mut session: Session = ron::de::from_reader(BufReader::new(File::open(path)?))?;
    ensure!(
        (1..=VERSION).contains(&session.version) && session.photos.len() <= 64,
        "Unsupported session version or too many photos"
    );
    for photo in &mut session.photos {
        photo.history.share_current_storage(&mut photo.edit);
    }
    Ok(session)
}

pub(crate) fn save(path: &Path, session: &Session) -> Result<()> {
    let (temp, file) = (0..1000)
        .find_map(|n| {
            let temp = path.with_extension(format!("ron.{}.{n}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => Some(Ok((temp, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .context("Cannot reserve session temporary file")??;
    let result = (|| -> Result<()> {
        let mut writer = BufWriter::with_capacity(128 * 1024, file);
        ron::Options::default().to_io_writer_pretty(&mut writer, session, Default::default())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

// Store growing action vectors as shared prefixes plus new suffixes, rather than repeating
// every instruction list in every saved undo snapshot.
mod compact_history {
    use super::*;
    use crate::{
        cleanup::{CloneStamp, PatchStroke},
        geometry::WarpStroke,
        shared::SharedVec,
    };
    use serde::{Deserializer, Serializer, de::Error};

    #[derive(Serialize, Deserialize)]
    struct Sequence<T> {
        base: Option<usize>,
        prefix: usize,
        suffix: Vec<T>,
    }
    #[derive(Serialize, Deserialize)]
    struct Snapshot {
        settings: Settings,
        preset: Option<String>,
        strokes: usize,
        warps: usize,
        clones: usize,
        patches: usize,
    }
    #[derive(Serialize, Deserialize)]
    struct Packed {
        past: Vec<Snapshot>,
        future: Vec<Snapshot>,
        strokes: Vec<Sequence<Stroke>>,
        warps: Vec<Sequence<WarpStroke>>,
        clones: Vec<Sequence<CloneStamp>>,
        patches: Vec<Sequence<PatchStroke>>,
    }
    fn append<T: Clone + PartialEq>(
        values: &SharedVec<T>,
        originals: &mut Vec<SharedVec<T>>,
        sequences: &mut Vec<Sequence<T>>,
    ) -> usize {
        if let Some(index) = originals
            .iter()
            .position(|v| v.same_storage(values) || **v == **values)
        {
            return index;
        }
        let base = originals.len().checked_sub(1);
        let prefix = base
            .map(|i| {
                originals[i]
                    .iter()
                    .zip(values.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .unwrap_or(0);
        sequences.push(Sequence {
            base,
            prefix,
            suffix: values[prefix..].to_vec(),
        });
        originals.push(values.clone());
        originals.len() - 1
    }
    fn expand<T: Clone>(sequences: Vec<Sequence<T>>) -> Result<Vec<SharedVec<T>>, String> {
        let mut result: Vec<SharedVec<T>> = Vec::new();
        for sequence in sequences {
            let mut values = if let Some(base) = sequence.base {
                let base = result.get(base).ok_or("Invalid history vector reference")?;
                base.get(..sequence.prefix)
                    .ok_or("Invalid history vector prefix")?
                    .to_vec()
            } else {
                if sequence.prefix != 0 {
                    return Err("Invalid history prefix without a base".into());
                }
                Vec::new()
            };
            values.extend(sequence.suffix);
            result.push(values.into());
        }
        Ok(result)
    }
    pub fn serialize<S: Serializer>(history: &History, serializer: S) -> Result<S::Ok, S::Error> {
        let mut packed = Packed {
            past: vec![],
            future: vec![],
            strokes: vec![],
            warps: vec![],
            clones: vec![],
            patches: vec![],
        };
        let (mut strokes, mut warps, mut clones, mut patches) = (vec![], vec![], vec![], vec![]);
        let mut snapshot = |edit: &Edit| Snapshot {
            settings: edit.settings.clone(),
            preset: edit.preset.clone(),
            strokes: append(&edit.strokes, &mut strokes, &mut packed.strokes),
            warps: append(&edit.warps, &mut warps, &mut packed.warps),
            clones: append(&edit.clones, &mut clones, &mut packed.clones),
            patches: append(&edit.patches, &mut patches, &mut packed.patches),
        };
        let (past, future) = history.snapshots();
        packed.past = past.map(&mut snapshot).collect();
        packed.future = future.map(snapshot).collect();
        packed.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<History, D::Error> {
        let packed = Packed::deserialize(deserializer)?;
        if packed.past.len() + packed.future.len() > 100 {
            return Err(D::Error::custom("Too many history snapshots"));
        }
        let strokes = expand(packed.strokes).map_err(D::Error::custom)?;
        let warps = expand(packed.warps).map_err(D::Error::custom)?;
        let clones = expand(packed.clones).map_err(D::Error::custom)?;
        let patches = expand(packed.patches).map_err(D::Error::custom)?;
        let snapshot = |saved: Snapshot| -> Result<Edit, D::Error> {
            Ok(Edit {
                settings: saved.settings,
                preset: saved.preset,
                strokes: strokes
                    .get(saved.strokes)
                    .ok_or_else(|| D::Error::custom("Invalid stroke history"))?
                    .clone(),
                warps: warps
                    .get(saved.warps)
                    .ok_or_else(|| D::Error::custom("Invalid liquify history"))?
                    .clone(),
                clones: clones
                    .get(saved.clones)
                    .ok_or_else(|| D::Error::custom("Invalid clone history"))?
                    .clone(),
                patches: patches
                    .get(saved.patches)
                    .ok_or_else(|| D::Error::custom("Invalid patch history"))?
                    .clone(),
            })
        };
        Ok(History::from_snapshots(
            packed
                .past
                .into_iter()
                .map(&snapshot)
                .collect::<Result<_, _>>()?,
            packed
                .future
                .into_iter()
                .map(snapshot)
                .collect::<Result<_, _>>()?,
        ))
    }
}

#[derive(Clone)]
pub(crate) struct RecoveryCandidate {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub bytes: u64,
}
pub(crate) struct RecoveryStore {
    pub path: PathBuf,
    lease: Option<File>,
}

fn recovery_directory() -> PathBuf {
    if let Some(root) = std::env::var_os("ASTRA_RECOVERY_DIR") {
        return root.into();
    }
    let root = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_STATE_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    root.join("AstraRetouch").join("recovery")
}
fn lease_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options
}
impl RecoveryStore {
    pub fn new() -> Result<Self> {
        Self::in_directory(&recovery_directory())
    }
    pub(crate) fn in_directory(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("workspace-{}-{stamp}.ron", std::process::id()));
        let lease = lease_options()
            .create_new(true)
            .open(path.with_extension("lock"))?;
        Ok(Self {
            path,
            lease: Some(lease),
        })
    }
}
impl Drop for RecoveryStore {
    fn drop(&mut self) {
        drop(self.lease.take());
        let _ = fs::remove_file(self.path.with_extension("lock"));
    }
}
pub(crate) fn recovery_candidates() -> Vec<RecoveryCandidate> {
    candidates_in(&recovery_directory())
}
fn candidates_in(directory: &Path) -> Vec<RecoveryCandidate> {
    let mut candidates = Vec::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return candidates;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ron") {
            continue;
        }
        let lock = path.with_extension("lock");
        if lock.exists() && lease_options().open(&lock).is_err() {
            continue;
        }
        if let Ok(metadata) = entry.metadata() {
            candidates.push(RecoveryCandidate {
                path,
                modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                bytes: metadata.len(),
            });
        }
    }
    candidates.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    candidates
}
pub(crate) fn read_recovery(path: &Path) -> Result<Session> {
    read(path).or_else(|primary| {
        read(&path.with_extension("previous")).with_context(|| {
            format!(
                "Latest recovery was invalid ({primary}); previous recovery is also unavailable"
            )
        })
    })
}
pub(crate) fn discard_recovery(path: &Path) -> Result<()> {
    for path in [
        path.to_path_buf(),
        path.with_extension("previous"),
        path.with_extension("lock"),
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
// A queued autosave can finish after the final shutdown snapshot. Under this lock, older
// generations cannot replace the final state. Each process writes its own independent path.
static RECOVERY_WRITES: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());
pub(crate) fn save_recovery(path: &Path, session: &Session, generation: u64) -> Result<()> {
    let mut writes = RECOVERY_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    if writes
        .iter()
        .any(|(p, saved)| p == path && *saved >= generation)
    {
        return Ok(());
    }
    if path.is_file() {
        let previous_temp = path.with_extension(format!("previous.{}.tmp", std::process::id()));
        fs::copy(path, &previous_temp)?;
        OpenOptions::new()
            .write(true)
            .open(&previous_temp)?
            .sync_all()?;
        fs::rename(previous_temp, path.with_extension("previous"))?;
    }
    save(path, session)?;
    if let Some((_, saved)) = writes.iter_mut().find(|(p, _)| p == path) {
        *saved = generation;
    } else {
        writes.push((path.to_path_buf(), generation));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cleanup::{CloneStamp, PatchStroke},
        engine::Target,
        geometry::WarpStroke,
    };
    fn fixture() -> Session {
        Session {
            version: VERSION,
            current: 0,
            compare: true,
            split: 0.38,
            clean_shutdown: false,
            photos: vec![SavedPhoto {
                path: Some(PathBuf::from("original.jpg")),
                edit: Edit::default(),
                rating: 4,
                segmentation: None,
                view: View::default(),
                crop_center: [0.6, 0.4],
                history: History::default(),
                selected: false,
            }],
        }
    }
    #[test]
    fn session_replacement_keeps_snapshots_and_round_trips_old_vector_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.ron");
        fs::write(&path, "previous content").unwrap();
        let mut session = fixture();
        session.photos[0].rating = 2;
        save(&path, &session).unwrap();
        session.photos[0].edit.settings.exposure = 1.0;
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.photos[0].edit.settings.exposure, 0.0);
        assert_eq!(loaded.photos[0].rating, 2);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn version_one_sessions_load_with_empty_history_and_workspace_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.ron");
        fs::write(
            &path,
            format!(
                "(version:1,photos:[(path:None,edit:{},rating:2)])",
                ron::to_string(&Edit::default()).unwrap()
            ),
        )
        .unwrap();
        let restored = read(&path).unwrap();
        assert_eq!(restored.photos[0].history.depths(), (0, 0));
        assert!(restored.photos[0].selected);
        assert_eq!(restored.current, 0);
        assert_eq!(restored.split, 0.5);
        assert!(!restored.compare);
    }
    #[test]
    fn all_local_tools_and_both_history_branches_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("complete.ron");
        let mut session = fixture();
        let photo = &mut session.photos[0];
        let mut expected = Vec::new();
        for step in 0..40 {
            photo.history.record(photo.edit.clone());
            photo.edit.settings.exposure = step as f32 * 0.03;
            photo.edit.strokes.push(Stroke {
                target: Target::Heal,
                center: [0.2, step as f32 * 0.01],
                radius: 0.01,
                erase: false,
                softness: 0.7,
            });
            photo.edit.warps.push(WarpStroke {
                center: [0.3, 0.4],
                delta: [step as f32 * 0.001, 0.0],
                radius: 0.02,
                softness: 0.65,
                strength: 80.0,
            });
            photo.edit.clones.push(CloneStamp {
                center: [0.4, 0.5],
                source: [0.5, 0.6],
                radius: 0.025,
                softness: 0.6,
                strength: 70.0,
            });
            photo.edit.patches.push(PatchStroke {
                boundary: vec![[0.1, 0.2], [0.3, 0.2], [0.2, 0.3]],
                offset: [0.2, 0.1],
                softness: 0.5,
                strength: 65.0,
            });
            expected.push(photo.edit.clone());
        }
        for _ in 0..12 {
            assert!(photo.history.undo(&mut photo.edit));
        }
        assert_eq!(photo.history.depths(), (28, 12));
        save(&path, &session).unwrap();
        let mut restored = read(&path).unwrap();
        assert!(restored.compare);
        assert_eq!(restored.split, 0.38);
        assert!(!restored.photos[0].selected);
        assert_eq!(restored.photos[0].crop_center, [0.6, 0.4]);
        let photo = &mut restored.photos[0];
        assert_eq!(photo.history.depths(), (28, 12));
        assert_eq!(photo.edit, expected[27]);
        for expected in &expected[28..] {
            assert!(photo.history.redo(&mut photo.edit));
            assert_eq!(photo.edit, *expected);
        }
        for expected in expected[..39].iter().rev() {
            assert!(photo.history.undo(&mut photo.edit));
            assert_eq!(photo.edit, *expected);
        }
        assert!(photo.history.undo(&mut photo.edit));
        assert_eq!(photo.edit, Edit::default());
        assert!(!photo.history.can_undo());
    }
    #[test]
    fn long_stroke_sessions_use_prefix_storage_and_share_unchanged_vectors_on_load() {
        let mut session = fixture();
        let photo = &mut session.photos[0];
        for step in 0..100 {
            photo.history.record(photo.edit.clone());
            for index in 0..40 {
                photo.edit.strokes.push(Stroke {
                    target: Target::Heal,
                    center: [step as f32 / 100., index as f32 / 40.],
                    radius: 0.01,
                    erase: false,
                    softness: 0.7,
                });
            }
        }
        let raw_snapshots: Vec<_> = photo.history.snapshots().0.cloned().collect();
        let repeated_size = ron::to_string(&raw_snapshots).unwrap().len();
        let saved = ron::to_string(&session).unwrap();
        assert!(
            saved.len() < repeated_size / 8,
            "Compact {} vs repeated {}",
            saved.len(),
            repeated_size
        );
        let mut restored: Session = ron::from_str(&saved).unwrap();
        assert_eq!(restored.photos[0].history.depths(), (100, 0));
        let photo = &mut restored.photos[0];
        for _ in 0..100 {
            assert!(photo.history.undo(&mut photo.edit));
        }
        assert_eq!(photo.edit.strokes.len(), 0);
        let mut stable = fixture();
        stable.photos[0].edit.strokes = raw_snapshots.last().unwrap().strokes.clone();
        for step in 0..10 {
            let p = &mut stable.photos[0];
            p.history.record(p.edit.clone());
            p.edit.settings.exposure = step as f32;
        }
        let restored: Session = ron::from_str(&ron::to_string(&stable).unwrap()).unwrap();
        let snapshots: Vec<_> = restored.photos[0].history.snapshots().0.collect();
        assert!(snapshots[0].strokes.same_storage(&snapshots[9].strokes));
    }
    #[test]
    fn corrupt_latest_recovery_falls_back_and_stale_jobs_cannot_overwrite_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::in_directory(directory.path()).unwrap();
        let mut session = fixture();
        save_recovery(&store.path, &session, 1).unwrap();
        session.photos[0].edit.settings.exposure = 1.0;
        save_recovery(&store.path, &session, 2).unwrap();
        assert_eq!(
            read_recovery(&store.path).unwrap().photos[0]
                .edit
                .settings
                .exposure,
            1.0
        );
        fs::write(&store.path, "interrupted or corrupted write").unwrap();
        assert_eq!(
            read_recovery(&store.path).unwrap().photos[0]
                .edit
                .settings
                .exposure,
            0.0
        );
        session.clean_shutdown = true;
        session.photos[0].edit.settings.exposure = 2.0;
        save_recovery(&store.path, &session, 4).unwrap();
        session.clean_shutdown = false;
        session.photos[0].edit.settings.exposure = -1.0;
        save_recovery(&store.path, &session, 3).unwrap();
        let restored = read_recovery(&store.path).unwrap();
        assert!(restored.clean_shutdown);
        assert_eq!(restored.photos[0].edit.settings.exposure, 2.0);
    }
    #[cfg(windows)]
    #[test]
    fn recovery_discovery_excludes_live_workspaces_and_discard_removes_all_generations() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::in_directory(directory.path()).unwrap();
        save_recovery(&store.path, &fixture(), 1).unwrap();
        save_recovery(&store.path, &fixture(), 2).unwrap();
        assert!(candidates_in(directory.path()).is_empty());
        let path = store.path.clone();
        drop(store);
        assert_eq!(candidates_in(directory.path()).len(), 1);
        discard_recovery(&path).unwrap();
        assert!(candidates_in(directory.path()).is_empty());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    #[test]
    fn invalid_history_references_are_rejected() {
        let mut session = fixture();
        session.photos[0].history.record(Edit::default());
        let saved = ron::to_string(&session)
            .unwrap()
            .replacen("strokes:0", "strokes:999", 1);
        assert!(ron::from_str::<Session>(&saved).is_err());
    }
    #[test]
    fn large_mutated_instruction_histories_stay_bounded_and_keep_the_nearest_steps() {
        let mut edit = Edit {
            strokes: vec![
                Stroke {
                    target: Target::Heal,
                    center: [0.5, 0.5],
                    radius: 0.01,
                    erase: false,
                    softness: 0.7
                };
                100_000
            ]
            .into(),
            ..Default::default()
        };
        let mut history = History::default();
        for step in 0..60 {
            history.record(edit.clone());
            edit.strokes[0].center[0] = step as f32 / 100.0;
        }
        assert!(history.depths().0 < 60);
        assert!(history.depths().0 >= 30);
        assert!(history.instruction_storage_bytes() <= 64 * 1024 * 1024);
        assert!(history.undo(&mut edit));
        assert_eq!(edit.strokes[0].center[0], 0.58);
        assert!(history.redo(&mut edit));
        assert_eq!(edit.strokes[0].center[0], 0.59);
        assert!(history.instruction_storage_bytes() <= 64 * 1024 * 1024);
    }
}
