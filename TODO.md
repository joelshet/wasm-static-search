# To do

- **Win back index build time for `region+1` fields.** Counting region words made `load_index` take 270 ms in Chrome, up from 224 ms, on an index of 57,531 places. Nearly all of it is the pass in `Index::new` that reads 251,000 region words and creates 9,373 terms only regions hold. Keystroke times didn't move, so whatever is tried here should be checked against them too.
- **Win back the wasm size of `load_pages` builds.** They grew from 151 KB to 155 KB for add-only fields, which page search never uses. Putting add-only fields behind a cargo feature would remove that.
