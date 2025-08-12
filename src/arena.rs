use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;

/// Thin wrapper so we can swap underlying container later if needed.
pub struct ArenaVec<'a, T> {
    pub arena: &'a Bump,
    pub inner: BumpVec<'a, T>,
}

impl<'a, T> ArenaVec<'a, T> {
    pub fn with_capacity_in(cap: usize, arena: &'a Bump) -> Self {
        Self {
            arena,
            inner: BumpVec::with_capacity_in(cap, arena),
        }
    }

    pub fn push(&mut self, item: T) {
        self.inner.push(item)
    }

    pub fn into_inner(self) -> BumpVec<'a, T> {
        self.inner
    }

    pub fn len(&self) -> usize { self.inner.len() }
}

impl<'a, T> IntoIterator for ArenaVec<'a, T> {
    type Item = T;
    type IntoIter = <BumpVec<'a, T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}