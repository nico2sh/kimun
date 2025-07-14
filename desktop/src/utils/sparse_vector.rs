use std::collections::HashMap;

pub struct SparseVector<T> {
    data: HashMap<usize, T>,
    size: usize,
}

impl<T> SparseVector<T> {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            size: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: HashMap::with_capacity(capacity),
            size: 0,
        }
    }

    pub fn insert(&mut self, index: usize, value: T) -> Option<T> {
        self.size = self.size.max(index + 1);
        self.data.insert(index, value)
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.data.get(&index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.data.get_mut(&index)
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        let result = self.data.remove(&index);
        // Update size if we removed the last element
        if index + 1 == self.size {
            self.recalculate_size();
        }
        result
    }

    /// Truncates the vector to the specified length.
    /// All elements at indices >= len will be removed.
    /// If len >= current length, no operation is performed.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.size {
            return; // Nothing to truncate
        }

        // Remove all elements with indices >= len
        self.data.retain(|&index, _| index < len);

        // Update the size
        self.size = len;
    }

    /// Alternative truncate implementation that returns removed elements
    /// for cases where you need to know what was removed
    pub fn truncate_with_removed(&mut self, len: usize) -> Vec<(usize, T)> {
        if len >= self.size {
            return Vec::new(); // Nothing to truncate
        }

        let mut removed = Vec::new();

        // Collect indices to remove (can't modify HashMap while iterating)
        let indices_to_remove: Vec<usize> = self
            .data
            .keys()
            .filter(|&&index| index >= len)
            .copied()
            .collect();

        // Remove elements and collect them
        for index in indices_to_remove {
            if let Some(value) = self.data.remove(&index) {
                removed.push((index, value));
            }
        }

        // Update the size
        self.size = len;

        // Sort by index for consistent ordering
        removed.sort_by_key(|(index, _)| *index);
        removed
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.data.iter().map(|(&k, v)| (k, v))
    }

    pub fn contains_key(&self, index: usize) -> bool {
        self.data.contains_key(&index)
    }

    /// Recalculates the size based on the maximum index in the HashMap
    /// Used internally when removing elements that might affect the size
    fn recalculate_size(&mut self) {
        self.size = self
            .data
            .keys()
            .max()
            .map(|&max_index| max_index + 1)
            .unwrap_or(0);
    }

    /// Returns the number of actual elements stored (non-None values)
    pub fn count_elements(&self) -> usize {
        self.data.len()
    }

    /// Clears all elements
    pub fn clear(&mut self) {
        self.data.clear();
        self.size = 0;
    }
}

// Implement Default trait for convenience
impl<T> Default for SparseVector<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_basic() {
        let mut vec = SparseVector::new();
        vec.insert(0, "a");
        vec.insert(2, "c");
        vec.insert(5, "f");
        vec.insert(10, "k");

        assert_eq!(vec.len(), 11);
        assert_eq!(vec.count_elements(), 4);

        // Truncate to length 6
        vec.truncate(6);

        assert_eq!(vec.len(), 6);
        assert_eq!(vec.count_elements(), 3); // Only elements at 0, 2, 5 remain
        assert_eq!(vec.get(0), Some(&"a"));
        assert_eq!(vec.get(2), Some(&"c"));
        assert_eq!(vec.get(5), Some(&"f"));
        assert_eq!(vec.get(10), None); // This was removed
    }

    #[test]
    fn test_truncate_no_op() {
        let mut vec = SparseVector::new();
        vec.insert(0, "a");
        vec.insert(2, "c");

        let original_len = vec.len();
        let original_count = vec.count_elements();

        // Truncate to same or larger length should do nothing
        vec.truncate(5);

        assert_eq!(vec.len(), original_len);
        assert_eq!(vec.count_elements(), original_count);
        assert_eq!(vec.get(0), Some(&"a"));
        assert_eq!(vec.get(2), Some(&"c"));
    }

    #[test]
    fn test_truncate_with_removed() {
        let mut vec = SparseVector::new();
        vec.insert(1, 10);
        vec.insert(3, 30);
        vec.insert(5, 50);
        vec.insert(7, 70);

        let removed = vec.truncate_with_removed(4);

        assert_eq!(vec.len(), 4);
        assert_eq!(vec.count_elements(), 2); // Only 1 and 3 remain
        assert_eq!(removed, vec![(5, 50), (7, 70)]);
        assert_eq!(vec.get(1), Some(&10));
        assert_eq!(vec.get(3), Some(&30));
        assert_eq!(vec.get(5), None);
        assert_eq!(vec.get(7), None);
    }

    #[test]
    fn test_truncate_to_zero() {
        let mut vec = SparseVector::new();
        vec.insert(0, "a");
        vec.insert(5, "f");

        vec.truncate(0);

        assert_eq!(vec.len(), 0);
        assert_eq!(vec.count_elements(), 0);
        assert!(vec.is_empty());
    }

    #[test]
    fn test_truncate_empty_vector() {
        let mut vec: SparseVector<i32> = SparseVector::new();

        vec.truncate(5);

        assert_eq!(vec.len(), 0);
        assert_eq!(vec.count_elements(), 0);
        assert!(vec.is_empty());
    }
}
