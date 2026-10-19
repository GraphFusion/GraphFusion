//! ASCII-case-insensitive names. The first spelling stays stored.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) struct CaseFoldMap<V> {
    inner: BTreeMap<String, V>,
}

impl<V> Default for CaseFoldMap<V> {
    fn default() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }
}

impl<V> CaseFoldMap<V> {
    pub(super) fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    fn existing(&self, key: &str) -> Option<&String> {
        self.inner
            .keys()
            .find(|stored| stored.eq_ignore_ascii_case(key))
    }

    pub(super) fn get(&self, key: &str) -> Option<&V> {
        self.inner.get(self.existing(key)?)
    }

    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        let stored = self.existing(key)?.clone();
        self.inner.get_mut(&stored)
    }
    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.inner.values()
    }
    pub(super) fn clear(&mut self) {
        self.inner.clear();
    }

    pub(super) fn contains_key(&self, key: &str) -> bool {
        self.existing(key).is_some()
    }

    pub(super) fn insert(&mut self, key: impl AsRef<str>, value: V) -> Option<V> {
        let key = key.as_ref();
        let stored = self
            .existing(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned());
        self.inner.insert(stored, value)
    }

    pub(super) fn remove(&mut self, key: &str) -> Option<V> {
        let stored = self.existing(key)?.clone();
        self.inner.remove(&stored)
    }

    pub(super) fn retain(&mut self, f: impl FnMut(&String, &mut V) -> bool) {
        self.inner.retain(f);
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.inner.iter()
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &String> {
        self.inner.keys()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<V> IntoIterator for CaseFoldMap<V> {
    type Item = (String, V);
    type IntoIter = std::collections::btree_map::IntoIter<String, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, V> IntoIterator for &'a CaseFoldMap<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, String, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<V> From<BTreeMap<String, V>> for CaseFoldMap<V> {
    fn from(inner: BTreeMap<String, V>) -> Self {
        let mut map = Self::new();
        map.extend(inner);
        map
    }
}

impl<V> Extend<(String, V)> for CaseFoldMap<V> {
    fn extend<I: IntoIterator<Item = (String, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<V> FromIterator<(String, V)> for CaseFoldMap<V> {
    fn from_iter<T: IntoIterator<Item = (String, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        map.extend(iter);
        map
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CaseFoldSet {
    inner: BTreeMap<String, ()>,
}

impl CaseFoldSet {
    pub(super) fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        self.inner
            .keys()
            .any(|stored| stored.eq_ignore_ascii_case(key))
    }

    pub(super) fn insert(&mut self, key: impl Into<String>) -> bool {
        let key = key.into();
        if self.contains(&key) {
            return false;
        }
        self.inner.insert(key, ());
        true
    }

    pub(super) fn retain(&mut self, mut f: impl FnMut(&String) -> bool) {
        self.inner.retain(|key, _| f(key));
    }
    pub(super) fn clear(&mut self) {
        self.inner.clear();
    }
}
