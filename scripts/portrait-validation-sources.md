# Portrait validation fixtures

These external fixtures are used for visual validation only. They are not training pairs and no model-quality gain is inferred from them. Downloaded files and generated native crops live in the ignored `output/portrait-validation/` directory.

| Fixture | Provenance and license | Download | SHA-256 |
| --- | --- | --- | --- |
| Eileen Collins / astronaut | NASA Great Images; public domain. [scikit-image's dataset documentation](https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.astronaut) documents the source and license. | [scikit-image v0.25.2](https://raw.githubusercontent.com/scikit-image/scikit-image/v0.25.2/skimage/data/astronaut.png) | `88431cd9653ccd539741b555fb0a46b61558b301d4110412b5bc28b5e3ea6cb5` |
| Grace Hopper, 1984 | James S. Davis / U.S. Navy official photograph, public domain. [Wikimedia provenance](https://commons.wikimedia.org/wiki/File:Grace_Hopper.jpg). Matplotlib distributes the same photograph as a sample crop. | [Matplotlib sample](https://raw.githubusercontent.com/matplotlib/matplotlib/main/lib/matplotlib/mpl-data/sample_data/grace_hopper.jpg) | `a8ca6d734765703b09728ab47fe59f473d93ae3967fc24c7c0288c3c7adb7130` |
| Macro face portrait | William Stitt, 2016, CC0 (published before Unsplash changed licenses). [Wikimedia file and license record](https://commons.wikimedia.org/wiki/File:Face_portrait_(Unsplash).jpg). | [Original 5184×3456](https://upload.wikimedia.org/wikipedia/commons/0/04/Face_portrait_%28Unsplash%29.jpg) | `7356daa8fd4ad53b946ce0036f06b014431dc89b7ae29ecd8ef18fc54edce6b5` |
| Studio demo | Existing generated `assets/demo-portrait.png` | bundled | local hash recorded in report |
| User portrait | Existing `example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG`, explicitly supplied by the user | local only | local hash recorded in report |

Native detail crops use untouched source pixels and remain at their original resolution. `runtime_validation` renders Auto Retouch, under-eye strengths 50/100, skin smoothing 100, and flyaway cleanup 100, saves eyes/hair-edge crops, and checks under-eye strength has a larger pixel effect at 100. That monotonic check does not substitute for human visual review.

For backend comparison, CPU and Burn run in separate processes with uncached AI, once with cold sessions and once with warm sessions. Each process reports Windows' peak working set, including model weights and intermediate inference buffers. It saves raw generated maps and an identical Auto Retouch overview for numerical comparison. Cold timings include loading weights and CPU compilation where the backend performs it. Native renders run after inference; their memory use is reported separately.
