use std::ops::Bound;

use crate::range_bounds_map::{IntoIter as RangeBoundsMapIntoIter, NiceRange};
use crate::{
	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap, TryFromBounds,
	TryFromBoundsError,
};

/// An ordered set of non-overlapping [`RangeBounds`] based on [`RangeBoundsMap`].
///
/// `I` is the generic type parameter for the [`Ord`] type the `K`
/// type is [`RangeBounds`] over.
///
/// `K` is the generic type parameter for the [`RangeBounds`]
/// implementing type in the set.
///
/// See [`RangeBoundsMap`] for more details.
///
/// [`RangeBounds`]: https://doc.rust-lang.org/std/ops/trait.RangeBounds.html
pub struct RangeBoundsSet<I, K> {
	inner: RangeBoundsMap<I, K, ()>,
}

impl<I, K> RangeBoundsSet<I, K>
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	/// See [`RangeBoundsMap::new()`] for more details.
	pub fn new() -> Self {
		RangeBoundsSet {
			inner: RangeBoundsMap::new(),
		}
	}
	/// See [`RangeBoundsMap::len()`] for more details.
	pub fn len(&self) -> usize {
		self.inner.len()
	}
	/// See [`RangeBoundsMap::is_empty()`] for more details.
	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}
	/// See [`RangeBoundsMap::overlaps()`] for more details.
	pub fn overlaps<Q>(&self, range: Q) -> bool
	where
		Q: NiceRange<I>,
	{
		self.inner.overlaps(range)
	}
	/// See [`RangeBoundsMap::overlapping()`] for more details.
	pub fn overlapping<Q>(
		&self,
		range: Q,
	) -> impl DoubleEndedIterator<Item = &K>
	where
		Q: NiceRange<I>,
	{
		self.inner.overlapping(range).map(first)
	}
	/// See [`RangeBoundsMap::contains_point()`] for more details.
	pub fn contains_point(&self, point: I) -> bool {
		self.inner.contains_point(point)
	}
	/// See [`RangeBoundsMap::iter()`] for more details.
	pub fn iter(&self) -> impl DoubleEndedIterator<Item = &K> {
		self.inner.iter().map(first)
	}
	/// See [`RangeBoundsMap::remove_overlapping()`] for more details.
	pub fn remove_overlapping<'a, Q>(
		&'a mut self,
		range: Q,
	) -> impl Iterator<Item = K> + '_
	where
		Q: NiceRange<I> + 'a,
	{
		self.inner.remove_overlapping(range).map(first)
	}
	/// See [`RangeBoundsMap::cut()`] for more details.
	pub fn cut<'a, Q>(
		&'a mut self,
		range: Q,
	) -> Result<
		impl Iterator<Item = (Bound<I>, Bound<I>)> + '_,
		TryFromBoundsError,
	>
	where
		Q: NiceRange<I> + 'a,
		K: TryFromBounds<I>,
	{
		self.inner.cut(range).map(|x| x.map(first))
	}
	/// See [`RangeBoundsMap::gaps()`] for more details.
	pub fn gaps<'a, Q>(
		&'a self,
		range: Q,
	) -> impl Iterator<Item = (Bound<I>, Bound<I>)> + '_
	where
		Q: NiceRange<I> + 'a,
	{
		self.inner.gaps(range)
	}
	/// See [`RangeBoundsMap::contains_range()`] for more details.
	pub fn contains_range<Q>(&self, range: Q) -> bool
	where
		Q: NiceRange<I>,
	{
		self.inner.contains_range(range)
	}
	/// See [`RangeBoundsMap::insert_strict()`] for more details.
	pub fn insert_strict(&mut self, range: K) -> Result<(), OverlapError> {
		self.inner.insert_strict(range, ())
	}
	/// See [`RangeBoundsMap::insert_merge_touching()`] for more details.
	pub fn insert_merge_touching(
		&mut self,
		range: K,
	) -> Result<K, OverlapOrTryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.inner.insert_merge_touching(range, ())
	}
	/// See [`RangeBoundsMap::insert_merge_overlapping()`] for more details.
	pub fn insert_merge_overlapping(
		&mut self,
		range: K,
	) -> Result<K, TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.inner.insert_merge_overlapping(range, ())
	}
	/// See [`RangeBoundsMap::insert_merge_touching_or_overlapping()`] for more details.
	pub fn insert_merge_touching_or_overlapping(
		&mut self,
		range: K,
	) -> Result<K, TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.inner.insert_merge_touching_or_overlapping(range, ())
	}
	/// See [`RangeBoundsMap::insert_overwrite()`] for more details.
	pub fn insert_overwrite(
		&mut self,
		range: K,
	) -> Result<(), TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.inner.insert_overwrite(range, ())
	}
	/// See [`RangeBoundsMap::first_entry()`] for more details.
	pub fn first(&self) -> Option<&K> {
		self.inner.first_entry().map(first)
	}
	/// See [`RangeBoundsMap::last_entry()`] for more details.
	pub fn last(&self) -> Option<&K> {
		self.inner.last_entry().map(first)
	}
	/// See [`RangeBoundsMap::from_slice_strict()`] for more details.
	pub fn from_slice_strict<const N: usize>(
		slice: [K; N],
	) -> Result<RangeBoundsSet<I, K>, OverlapError> {
		let mut set = RangeBoundsSet::new();
		for range in slice {
			set.insert_strict(range)?;
		}
		return Ok(set);
	}
}

// Helper Functions ==========================

fn first<A, B>((a, _): (A, B)) -> A {
	a
}

// Trait Impls ==========================

impl<I, K> IntoIterator for RangeBoundsSet<I, K> {
	type Item = K;
	type IntoIter = IntoIter<I, K>;
	fn into_iter(self) -> Self::IntoIter {
		return IntoIter {
			inner: self.inner.into_iter(),
		};
	}
}
/// An owning iterator over the entries of a [`RangeBoundsSet`].
///
/// This `struct` is created by the [`into_iter`] method on
/// [`RangeBoundsSet`] (provided by the [`IntoIterator`] trait). See
/// its documentation for more.
///
/// [`into_iter`]: IntoIterator::into_iter
/// [`IntoIterator`]: core::iter::IntoIterator
pub struct IntoIter<I, K> {
	inner: RangeBoundsMapIntoIter<I, K, ()>,
}
impl<I, K> Iterator for IntoIter<I, K> {
	type Item = K;
	fn next(&mut self) -> Option<Self::Item> {
		self.inner.next().map(first)
	}
}
