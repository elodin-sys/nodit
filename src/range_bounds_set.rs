/*
Copyright 2022 James Forster

This file is part of range_bounds_map.

range_bounds_map is free software: you can redistribute it and/or
modify it under the terms of the GNU General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

range_bounds_map is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
General Public License for more details.

You should have received a copy of the GNU General Public License
along with range_bounds_map. If not, see <https://www.gnu.org/licenses/>.
*/

use std::fmt::Debug;
use std::ops::{Bound, RangeBounds};

use labels::{tested, trivial};
use serde::{Deserialize, Serialize};

use crate::{
	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap, TryFromBounds,
	TryFromBoundsError,
};

/// An ordered set of [`RangeBounds`] based on [`RangeBoundsMap`].
///
/// `I` is the generic type parameter for the [`Ord`] type the `K`
/// type is [`RangeBounds`] over.
///
/// `K` is the generic type parameter for the [`RangeBounds`]
/// implementing type in the set.
///
/// # Examples
/// ```
/// use range_bounds_map::RangeBoundsSet;
///
/// // Make a new set
/// let mut set =
/// 	RangeBoundsSet::try_from([4..8, 8..18, 20..100]).unwrap();
///
/// if set.contains_point(&99) {
/// 	println!("Set contains value at 99 :)");
/// }
///
/// // Iterate over the entries in the set
/// for range in set.iter() {
/// 	println!("{range:?}");
/// }
/// ```
/// Example using a custom [`RangeBounds`] type:
/// ```
/// use std::ops::{Bound, RangeBounds};
///
/// use ordered_float::NotNan;
/// use range_bounds_map::RangeBoundsSet;
///
/// // An Exlusive-Exlusive range of [`f32`]s not provided by any
/// // std::ops ranges
/// // We use [`ordered_float::NotNan`]s as the inner type must be Ord
/// // similar to a normal [`BTreeSet`]
/// #[derive(Debug, PartialEq)]
/// struct ExEx {
/// 	start: NotNan<f32>,
/// 	end: NotNan<f32>,
/// }
/// # impl ExEx {
/// #    fn new(start: f32, end: f32) -> ExEx {
/// #        ExEx {
/// #            start: NotNan::new(start).unwrap(),
/// #            end: NotNan::new(end).unwrap(),
/// #        }
/// #    }
/// # }
///
/// // Implement RangeBounds<f32> on our new type
/// impl RangeBounds<NotNan<f32>> for ExEx {
/// 	fn start_bound(&self) -> Bound<&NotNan<f32>> {
/// 		Bound::Excluded(&self.start)
/// 	}
/// 	fn end_bound(&self) -> Bound<&NotNan<f32>> {
/// 		Bound::Excluded(&self.end)
/// 	}
/// }
///
/// // Now we can make a [`RangeBoundsSet`] of [`ExEx`]s
/// let mut set = RangeBoundsSet::new();
///
/// set.insert_platonic(ExEx::new(0.0, 5.0)).unwrap();
/// set.insert_platonic(ExEx::new(5.0, 7.5)).unwrap();
///
/// assert_eq!(set.contains_point(&NotNan::new(5.0).unwrap()), false);
///
/// assert_eq!(set.get_at_point(&NotNan::new(9.0).unwrap()), None);
/// assert_eq!(
/// 	set.get_at_point(&NotNan::new(7.0).unwrap()),
/// 	Some(&ExEx::new(5.0, 7.5))
/// );
/// ```
///
/// [`RangeBounds`]: https://doc.rust-lang.org/std/ops/trait.RangeBounds.html
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct RangeBoundsSet<I, K>
where
	I: PartialOrd,
{
	map: RangeBoundsMap<I, K, ()>,
}

impl<I, K> RangeBoundsSet<I, K>
where
	K: RangeBounds<I>,
	I: Ord + Clone,
{
	/// Makes a new, empty `RangeBoundsSet`.
	///
	/// # Examples
	/// ```
	/// use std::ops::Range;
	///
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set: RangeBoundsSet<u8, Range<u8>> =
	/// 	RangeBoundsSet::new();
	/// ```
	#[trivial]
	pub fn new() -> Self {
		RangeBoundsSet {
			map: RangeBoundsMap::new(),
		}
	}

	/// Returns the number of `RangeBounds` in the set.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set = RangeBoundsSet::new();
	///
	/// assert_eq!(range_bounds_set.len(), 0);
	/// range_bounds_set.insert_platonic(0..1).unwrap();
	/// assert_eq!(range_bounds_set.len(), 1);
	/// ```
	#[trivial]
	pub fn len(&self) -> usize {
		self.map.len()
	}

	/// Adds a new `RangeBounds` to the set without modifying other
	/// `RangeBounds` in the set.
	///
	/// If the new `RangeBounds` overlaps one or more `RangeBounds`
	/// already in the set rather than just touching, then an
	/// [`OverlapError`] is returned and the set is not updated.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::{OverlapError, RangeBoundsSet};
	///
	/// let mut range_bounds_set = RangeBoundsSet::new();
	///
	/// assert_eq!(range_bounds_set.insert_platonic(5..10), Ok(()));
	/// assert_eq!(
	/// 	range_bounds_set.insert_platonic(5..10),
	/// 	Err(OverlapError)
	/// );
	/// assert_eq!(range_bounds_set.len(), 1);
	/// ```
	#[tested]
	pub fn insert_platonic(
		&mut self,
		range_bounds: K,
	) -> Result<(), OverlapError> {
		self.map.insert_platonic(range_bounds, ())
	}

	/// Returns `true` if the given `RangeBounds` overlaps any of the
	/// `RangeBounds` in the set.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set = RangeBoundsSet::new();
	///
	/// range_bounds_set.insert_platonic(5..10);
	///
	/// assert_eq!(range_bounds_set.overlaps(&(1..=3)), false);
	/// assert_eq!(range_bounds_set.overlaps(&(4..5)), false);
	///
	/// assert_eq!(range_bounds_set.overlaps(&(4..=5)), true);
	/// assert_eq!(range_bounds_set.overlaps(&(4..6)), true);
	/// ```
	#[trivial]
	pub fn overlaps<Q>(&self, range_bounds: &Q) -> bool
	where
		Q: RangeBounds<I>,
	{
		self.map.overlaps(range_bounds)
	}

	/// Returns an iterator over every `RangeBounds` in the set which
	/// overlap the given `range_bounds` in ascending order.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// let mut overlapping = range_bounds_set.overlapping(&(2..8));
	///
	/// assert_eq!(
	/// 	overlapping.collect::<Vec<_>>(),
	/// 	[(&(1..4)), (&(4..8))]
	/// );
	/// ```
	#[tested]
	pub fn overlapping<Q>(
		&self,
		range_bounds: &Q,
	) -> impl DoubleEndedIterator<Item = &K>
	where
		Q: RangeBounds<I>,
	{
		self.map.overlapping(range_bounds).map(|(key, _)| key)
	}

	/// Returns a reference to the `RangeBounds` in the set that
	/// overlaps the given point, if any.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// assert_eq!(range_bounds_set.get_at_point(&3), Some(&(1..4)));
	/// assert_eq!(range_bounds_set.get_at_point(&4), Some(&(4..8)));
	/// assert_eq!(range_bounds_set.get_at_point(&101), None);
	/// ```
	#[trivial]
	pub fn get_at_point(&self, point: &I) -> Option<&K> {
		self.map.get_entry_at_point(point).map(|(key, _)| key)
	}

	/// Returns `true` if the set contains a `RangeBounds` that
	/// overlaps the given point, and `false` if not.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// assert_eq!(range_bounds_set.contains_point(&3), true);
	/// assert_eq!(range_bounds_set.contains_point(&4), true);
	/// assert_eq!(range_bounds_set.contains_point(&101), false);
	/// ```
	#[trivial]
	pub fn contains_point(&self, point: &I) -> bool {
		self.map.contains_point(point)
	}

	/// Returns an iterator over every `RangeBounds` in the set in
	/// ascending order.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// let mut iter = range_bounds_set.iter();
	///
	/// assert_eq!(iter.next(), Some(&(1..4)));
	/// assert_eq!(iter.next(), Some(&(4..8)));
	/// assert_eq!(iter.next(), Some(&(8..100)));
	/// assert_eq!(iter.next(), None);
	/// ```
	#[trivial]
	pub fn iter(&self) -> impl DoubleEndedIterator<Item = &K> {
		self.map.iter().map(|(key, _)| key)
	}

	/// Removes every `RangeBounds` in the set which overlaps the
	/// given `range_bounds` and returns them in an iterator.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// let mut removed = range_bounds_set.remove_overlapping(&(2..8));
	///
	/// assert_eq!(removed.collect::<Vec<_>>(), [1..4, 4..8]);
	///
	/// assert_eq!(
	/// 	range_bounds_set.iter().collect::<Vec<_>>(),
	/// 	[&(8..100)]
	/// );
	/// ```
	#[tested]
	pub fn remove_overlapping<Q>(
		&mut self,
		range_bounds: &Q,
	) -> impl DoubleEndedIterator<Item = K>
	where
		Q: RangeBounds<I>,
	{
		self.map
			.remove_overlapping(range_bounds)
			.map(|(key, _)| key)
	}

	/// Cuts a given `RangeBounds` out of the set.
	///
	/// If the remaining `RangeBounds` left after the cut are not able
	/// to be created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::{RangeBoundsSet, TryFromBoundsError};
	///
	/// let mut base =
	/// 	RangeBoundsSet::try_from([1..4, 4..8, 8..100]).unwrap();
	///
	/// let after_cut =
	/// 	RangeBoundsSet::try_from([1..2, 40..100]).unwrap();
	///
	/// assert_eq!(base.cut(&(2..40)), Ok(()));
	/// assert_eq!(base, after_cut);
	/// assert_eq!(base.cut(&(60..=80)), Err(TryFromBoundsError));
	/// ```
	#[tested]
	pub fn cut<Q>(&mut self, range_bounds: &Q) -> Result<(), TryFromBoundsError>
	where
		Q: RangeBounds<I>,
		K: TryFromBounds<I>,
	{
		self.map.cut(range_bounds)
	}

	/// Returns an iterator of `(Bound<&I>, Bound<&I>)` over all the
	/// maximally-sized gaps in the set that are also within the given
	/// `outer_range_bounds`.
	///
	/// To get all possible gaps call `gaps()` with an unbounded
	/// `RangeBounds` such as `&(..)` or `&(Bound::Unbounded,
	/// Bound::Unbounded)`.
	///
	/// # Examples
	/// ```
	/// use std::ops::Bound;
	///
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..3, 5..7, 9..100]).unwrap();
	///
	/// let mut gaps = range_bounds_set.gaps(&(2..));
	///
	/// assert_eq!(
	/// 	gaps.collect::<Vec<_>>(),
	/// 	[
	/// 		(Bound::Included(&3), Bound::Excluded(&5)),
	/// 		(Bound::Included(&7), Bound::Excluded(&9)),
	/// 		(Bound::Included(&100), Bound::Unbounded)
	/// 	]
	/// );
	/// ```
	#[tested]
	pub fn gaps<'a, Q>(
		&'a self,
		outer_range_bounds: &'a Q,
	) -> impl Iterator<Item = (Bound<&I>, Bound<&I>)>
	where
		Q: RangeBounds<I>,
	{
		self.map.gaps(outer_range_bounds)
	}

	/// Returns `true` if the set covers every point in the given
	/// `RangeBounds`, and `false` if it doesn't.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..3, 5..8, 8..100]).unwrap();
	///
	/// assert_eq!(range_bounds_set.contains_range_bounds(&(1..3)), true);
	/// assert_eq!(
	/// 	range_bounds_set.contains_range_bounds(&(2..6)),
	/// 	false
	/// );
	/// assert_eq!(
	/// 	range_bounds_set.contains_range_bounds(&(6..50)),
	/// 	true
	/// );
	/// ```
	#[trivial]
	pub fn contains_range_bounds<Q>(&self, range_bounds: &Q) -> bool
	where
		Q: RangeBounds<I>,
	{
		self.map.contains_range_bounds(range_bounds)
	}

	/// Adds a new `RangeBounds` to the set and coalesces into other
	/// `RangeBounds` in the set which touch it.
	///
	/// If successful then a reference to the newly inserted
	/// `RangeBounds` is returned.
	///
	/// If the new `RangeBounds` overlaps one or more `RangeBounds`
	/// already in the set rather than just touching, then an
	/// [`OverlapError`] is returned and the set is not updated.
	///
	/// If the coalesced `RangeBounds` cannot be created with the
	/// [`TryFromBounds`] trait then a [`TryFromBoundsError`] will be
	/// returned.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::{
	/// 	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsSet,
	/// };
	///
	/// let mut range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4]).unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_touching(4..6),
	/// 	Ok(&(1..6))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_touching(4..8),
	/// 	Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_touching(10..16),
	/// 	Ok(&(10..16))
	/// );
	///
	/// assert_eq!(
	/// 	range_bounds_set.iter().collect::<Vec<_>>(),
	/// 	[&(1..6), &(10..16)]
	/// );
	/// ```
	#[tested]
	pub fn insert_coalesce_touching(
		&mut self,
		range_bounds: K,
	) -> Result<&K, OverlapOrTryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.map.insert_coalesce_touching(range_bounds, ())
	}

	/// Adds a new `RangeBounds` to the set and coalesces into other
	/// `RangeBounds` in the set which overlap it.
	///
	/// If successful then a reference to the newly inserted
	/// `RangeBounds` is returned.
	///
	/// If the coalesced `RangeBounds` cannot be created with the
	/// [`TryFromBounds`] trait then a [`TryFromBoundsError`] will be
	/// returned.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4]).unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_overlapping(-4..1),
	/// 	Ok(&(-4..1))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_overlapping(2..8),
	/// 	Ok(&(1..8))
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	range_bounds_set.insert_coalesce_overlapping(10..16),
	/// 	Ok(&(10..16))
	/// );
	///
	/// assert_eq!(
	/// 	range_bounds_set.iter().collect::<Vec<_>>(),
	/// 	[&(-4..1), &(1..8), &(10..16)]
	/// );
	/// ```
	#[tested]
	pub fn insert_coalesce_overlapping(
		&mut self,
		range_bounds: K,
	) -> Result<&K, TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.map.insert_coalesce_overlapping(range_bounds, ())
	}

	/// Adds a new `RangeBounds` to the set and coalesces into other
	/// `RangeBounds` in the set which touch or overlap it.
	///
	/// If successful then a reference to the newly inserted
	/// `RangeBounds` is returned.
	///
	/// If the coalesced `RangeBounds` cannot be created with the
	/// [`TryFromBounds`] trait then a [`TryFromBoundsError`] will be
	/// returned.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set =
	/// 	RangeBoundsSet::try_from([1..4]).unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	range_bounds_set
	/// 		.insert_coalesce_touching_or_overlapping(-4..1),
	/// 	Ok(&(-4..4))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	range_bounds_set
	/// 		.insert_coalesce_touching_or_overlapping(2..8),
	/// 	Ok(&(-4..8))
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	range_bounds_set
	/// 		.insert_coalesce_touching_or_overlapping(10..16),
	/// 	Ok(&(10..16))
	/// );
	///
	/// assert_eq!(
	/// 	range_bounds_set.iter().collect::<Vec<_>>(),
	/// 	[&(-4..8), &(10..16)]
	/// );
	/// ```
	#[tested]
	pub fn insert_coalesce_touching_or_overlapping(
		&mut self,
		range_bounds: K,
	) -> Result<&K, TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.map
			.insert_coalesce_touching_or_overlapping(range_bounds, ())
	}

	/// Adds a new `RangeBounds` to the set and overwrites any other
	/// `RangeBounds` that overlap the new `RangeBounds`.
	///
	/// This is equivalent to using [`RangeBoundsSet::cut()`]
	/// followed by [`RangeBoundsSet::insert_platonic()`].
	///
	/// If the remaining `RangeBounds` left after the cut are not able
	/// to be created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsSet;
	///
	/// let mut range_bounds_set =
	/// 	RangeBoundsSet::try_from([2..8]).unwrap();
	///
	/// assert_eq!(range_bounds_set.overwrite(4..6), Ok(()));
	///
	/// assert_eq!(
	/// 	range_bounds_set.iter().collect::<Vec<_>>(),
	/// 	[&(2..4), &(4..6), &(6..8)]
	/// );
	/// ```
	#[trivial]
	pub fn overwrite(
		&mut self,
		range_bounds: K,
	) -> Result<(), TryFromBoundsError>
	where
		K: TryFromBounds<I>,
	{
		self.map.overwrite(range_bounds, ())
	}
}

impl<const N: usize, I, K> TryFrom<[K; N]> for RangeBoundsSet<I, K>
where
	K: RangeBounds<I>,
	I: Ord + Clone,
{
	type Error = OverlapError;
	#[trivial]
	fn try_from(pairs: [K; N]) -> Result<Self, Self::Error> {
		let mut range_bounds_set = RangeBoundsSet::new();
		for range_bounds in pairs {
			range_bounds_set.insert_platonic(range_bounds)?;
		}

		return Ok(range_bounds_set);
	}
}
impl<I, K> TryFrom<Vec<K>> for RangeBoundsSet<I, K>
where
	K: RangeBounds<I>,
	I: Ord + Clone,
{
	type Error = OverlapError;
	#[trivial]
	fn try_from(pairs: Vec<K>) -> Result<Self, Self::Error> {
		let mut range_bounds_set = RangeBoundsSet::new();
		for range_bounds in pairs {
			range_bounds_set.insert_platonic(range_bounds)?;
		}

		return Ok(range_bounds_set);
	}
}
