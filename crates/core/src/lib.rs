//! Search any text by whatever you remember of it. Records come in as tab-separated text, a corpus brings its own
//! tables of word forms, synonyms and names, and the same code builds and searches the index in a command line
//! tool and in the browser.

pub mod bm25;
pub mod fast;
pub mod index;
pub mod json;
pub mod pages;
pub mod records;
pub mod split;
pub mod tables;
pub mod text;
