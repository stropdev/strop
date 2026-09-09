//! A persistent catalog: snapshots share structure instead of cloning a workspace
//! on every query. Leaf copies contain Arc handles, never filename/line strings.
use crate::Item;
use imbl::Vector;
use std::ops::Index;
use std::sync::Arc;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    items: Vector<Arc<Item>>,
}
impl Catalog {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&Item> {
        self.items.get(index).map(Arc::as_ref)
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Item> + DoubleEndedIterator {
        self.items.iter().map(Arc::as_ref)
    }
    pub fn append(&mut self, items: Vec<Item>) {
        self.items.extend(items.into_iter().map(Arc::new));
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
}
impl From<Vec<Item>> for Catalog {
    fn from(items: Vec<Item>) -> Self {
        Self {
            items: items.into_iter().map(Arc::new).collect(),
        }
    }
}
impl Index<usize> for Catalog {
    type Output = Item;
    fn index(&self, index: usize) -> &Item {
        &self.items[index]
    }
}
