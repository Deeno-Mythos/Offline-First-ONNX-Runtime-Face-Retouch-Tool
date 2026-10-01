//! Immutable snapshots share storage; painting copies only the vector being changed.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct SharedVec<T>(Arc<Vec<T>>);
impl<T: PartialEq> PartialEq for SharedVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.same_storage(other) || self.0.as_ref() == other.0.as_ref()
    }
}
impl<T> SharedVec<T> {
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl<T> Default for SharedVec<T> {
    fn default() -> Self {
        Self(Arc::new(Vec::new()))
    }
}
impl<T> From<Vec<T>> for SharedVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self(Arc::new(value))
    }
}
impl<T> FromIterator<T> for SharedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Vec::from_iter(iter).into()
    }
}
impl<T: Send> rayon::iter::FromParallelIterator<T> for SharedVec<T> {
    fn from_par_iter<I: rayon::iter::IntoParallelIterator<Item = T>>(iter: I) -> Self {
        use rayon::prelude::*;
        iter.into_par_iter().collect::<Vec<T>>().into()
    }
}
impl<T> Deref for SharedVec<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: Clone> DerefMut for SharedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}
impl<'a, T> IntoIterator for &'a SharedVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<T: Clone> IntoIterator for SharedVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}
impl<T: Serialize> Serialize for SharedVec<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for SharedVec<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_share_until_mutated_and_keep_the_existing_session_format() {
        let original: SharedVec<u32> = vec![1, 2, 3].into();
        let mut edit = original.clone();
        assert!(Arc::ptr_eq(&original.0, &edit.0));
        edit[1] = 9;
        assert_eq!(&**original, &[1, 2, 3]);
        assert_eq!(&**edit, &[1, 9, 3]);
        let text = ron::to_string(&edit).unwrap();
        assert_eq!(text, ron::to_string(&vec![1u32, 9, 3]).unwrap());
        assert_eq!(ron::from_str::<SharedVec<u32>>(&text).unwrap(), edit);
    }
}
