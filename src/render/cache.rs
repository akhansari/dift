use std::collections::VecDeque;

/// Worker-owned LRU with estimated byte and entry limits. Active results may also
/// be held by the UI; eviction never invalidates those references.
pub(super) struct Cache<T> {
    entries: VecDeque<(T, usize)>,
    bytes: usize,
    limit: usize,
}
impl<T> Cache<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(value, _)| value)
    }
    pub fn take(&mut self, predicate: impl FnMut(&T) -> bool) -> Option<T> {
        let index = self.iter().position(predicate)?;
        let (value, bytes) = self.entries.remove(index)?;
        self.bytes = self.bytes.saturating_sub(bytes);
        Some(value)
    }
    pub fn insert(&mut self, value: T, bytes: usize) {
        if bytes > self.limit {
            return;
        }
        while self.bytes.saturating_add(bytes) > self.limit || self.entries.len() >= 64 {
            if let Some((_, removed)) = self.entries.pop_back() {
                self.bytes = self.bytes.saturating_sub(removed);
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries.push_front((value, bytes));
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

pub(super) fn fingerprint(value: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exceeding_the_byte_budget_evicts_the_least_recent_entry() {
        // Given
        let mut cache = Cache::new(4);
        cache.insert("first", 2);
        cache.insert("second", 2);
        // When
        cache.insert("third", 2);
        // Then
        assert_eq!(
            cache.iter().copied().collect::<Vec<_>>(),
            vec!["third", "second"]
        );
        assert_eq!(cache.bytes(), 4);
    }
    #[test]
    fn an_oversized_entry_is_not_retained() {
        // Given
        let mut cache = Cache::new(4);
        // When
        cache.insert("large", 5);
        // Then
        assert_eq!(cache.iter().count(), 0);
        assert_eq!(cache.bytes(), 0);
    }
}
