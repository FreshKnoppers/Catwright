//! Tiny bounded memo caches for work the UI would otherwise redo every
//! frame: markdown parsing, line diffs and their stats. Entries are keyed
//! by a content hash; a full cache is dropped wholesale (cheaper and
//! safer than an LRU at these sizes — the next frame refills it).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

pub fn hash_str(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// A pair hash for `(Option<&str>, &str)` inputs (old/new of a diff).
pub fn hash_pair(old: Option<&str>, new: &str) -> u64 {
    let mut h = DefaultHasher::new();
    old.is_some().hash(&mut h);
    old.unwrap_or("").hash(&mut h);
    new.hash(&mut h);
    h.finish()
}

pub struct Memo<T> {
    map: Mutex<HashMap<u64, Arc<T>>>,
    cap: usize,
}

impl<T> Memo<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            cap,
        }
    }

    pub fn get_or_insert(&self, key: u64, make: impl FnOnce() -> T) -> Arc<T> {
        let mut map = self.map.lock().unwrap();
        if let Some(v) = map.get(&key) {
            return v.clone();
        }
        let value = Arc::new(make());
        if map.len() >= self.cap {
            map.clear();
        }
        map.insert(key, value.clone());
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_reuses_and_caps() {
        let memo: Memo<Vec<u8>> = Memo::new(2);
        let a = memo.get_or_insert(1, || vec![1]);
        let b = memo.get_or_insert(1, || vec![2]);
        assert!(Arc::ptr_eq(&a, &b), "the same key returns the same entry");
        assert_eq!(*b, vec![1], "the maker did not run again");
        memo.get_or_insert(2, || vec![3]);
        assert!(
            Arc::ptr_eq(&a, &memo.get_or_insert(1, || vec![9])),
            "reaching the cap alone does not clear"
        );
        memo.get_or_insert(3, || vec![4]);
        assert_eq!(
            *memo.get_or_insert(1, || vec![9]),
            vec![9],
            "exceeding the cap drops the whole cache"
        );
    }

    #[test]
    fn pair_hash_separates_none_from_empty_old() {
        assert_ne!(hash_pair(None, "x"), hash_pair(Some(""), "x"));
        assert_ne!(hash_pair(Some("a"), "b"), hash_pair(Some("a b"), ""));
    }
}
