/*
Copyright 2022 James Forster

This file is part of range_bounds_map.

range_bounds_map is free software: you can redistribute it and/or
modify it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

range_bounds_map is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with range_bounds_map. If not, see <https://www.gnu.org/licenses/>.
*/

use std::cmp::Ordering;
use std::fmt::{self, Debug};
use std::iter::once;
use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};

use btree_monstrousity::btree_map::{
	IntoIter as BTreeMapIntoIter, SearchBoundCustom,
};
use btree_monstrousity::BTreeMap;
use either::Either;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::bound_ord::DiscreteBoundOrd;
use crate::discrete_bounds::DiscreteBounds;
use crate::utils::{
	cmp_range_with_discrete_bound_ord, cut_range, flip_bound, is_valid_range,
	overlaps,
};

/// An ordered map of non-overlapping ranges based on [`BTreeMap`].
///
/// `I` is the generic type parameter for the [`Ord`] type the `K` type
/// is [`RangeBounds`] over.
///
/// `K` is the generic type parameter for the [`RangeBounds`]
/// implementing type stored as the keys in the map.
///
/// `V` is the generic type parameter for the values associated with the
/// keys in the map.
///
/// Phrasing it another way: `I` is the point type, `K` is the range type, and `V` is the value type.
///
/// # Examples
/// ```
/// use range_bounds_map::test_ranges::ie;
/// use range_bounds_map::RangeBoundsMap;
///
/// // Make a map of ranges to booleans
/// let mut map = RangeBoundsMap::from_slice_strict([
/// 	(ie(4, 8), false),
/// 	(ie(8, 18), true),
/// 	(ie(20, 100), false),
/// ])
/// .unwrap();
///
/// // Change a value in the map
/// *map.get_at_point_mut(7).unwrap() = true;
///
/// if map.contains_point(99) {
/// 	println!("Map contains value at 99 :)");
/// }
///
/// // Iterate over the entries in the map
/// for (range, value) in map.iter() {
/// 	println!("{range:?}, {value:?}");
/// }
/// ```
/// Example using a custom [`RangeBounds`] type:
/// ```
/// use std::ops::{Bound, RangeBounds};
///
/// use ordered_float::NotNan;
/// use range_bounds_map::RangeBoundsMap;
///
/// // An Exclusive-Exclusive range of [`f32`]s is not provided by any
/// // std::ops ranges.
///
/// // We use [`ordered_float::NotNan`]s as the inner type must be Ord
/// // similar to a normal [`BTreeMap`].
/// #[derive(Debug, Copy, Clone, PartialEq)]
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
/// // Now we can make a [`RangeBoundsMap`] of [`ExEx`]s to `i8`
/// let mut map = RangeBoundsMap::new();
///
/// map.insert_strict(ExEx::new(0.0, 5.0), 8).unwrap();
/// map.insert_strict(ExEx::new(5.0, 7.5), 32).unwrap();
///
/// assert_eq!(map.contains_point(NotNan::new(5.0).unwrap()), false);
///
/// assert_eq!(map.get_at_point(NotNan::new(9.0).unwrap()), None);
/// assert_eq!(
/// 	map.get_at_point(NotNan::new(7.0).unwrap()),
/// 	Some(&32)
/// );
///
/// assert_eq!(
/// 	map.get_entry_at_point(NotNan::new(2.0).unwrap()),
/// 	Ok((&ExEx::new(0.0, 5.0), &8))
/// );
/// ```
///
/// [`RangeBounds`]: https://doc.rust-lang.org/std/ops/trait.RangeBounds.html
/// [`BTreeMap`]: https://doc.rust-lang.org/std/collections/struct.BTreeMap.html
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeBoundsMap<I, K, V> {
	inner: BTreeMap<K, V>,
	phantom: PhantomData<I>,
}

/// An error type to represent a [`RangeBounds`] overlapping another
/// [`RangeBounds`] when it should not have.
#[derive(PartialEq, Debug)]
pub struct OverlapError;

/// An error type to represent a failed [`TryFromBounds`] within a
/// method.
///
/// There are several methods that return this error, and some of the
/// causes of this error can be very subtle, so here are some examples
/// showing all the reasons this error might be returned.
///
/// # Example with [`RangeBoundsMap::cut()`]
///
/// The first way you may recieve [`TryFromBoundsError`] is from
/// [`RangeBoundsMap::cut()`].
///
/// In this example we try to cut `ii(4, 6)` out of a `RangeBoundsMap`
/// that contains `ie(2, 8)`. If this was successful then the
/// `RangeBoundsMap` would hold `ie(2, 4)` and `(Bound::Exclusive(6),
/// Bound::Exclusive(8))`. However, since the `RangeBounds` type of
/// this `RangeBoundsMap` is `Range<{integer}>` the latter of the two
/// new `RangeBounds` is "unrepresentable", and hence will fail to be
/// created via [`TryFromBounds`] and [`RangeBoundsMap::cut()`] will
/// return Err(TryFromBoundsError).
///
/// ```
/// use range_bounds_map::test_ranges::{ie_strict, ii};
/// use range_bounds_map::{RangeBoundsMap, TryFromBoundsError};
///
/// let mut map =
/// 	RangeBoundsMap::from_slice_strict([(ie_strict(2, 8), true)])
/// 		.unwrap();
///
/// assert!(map.cut(ii(4, 6)).is_err());
/// ```
///
/// # Example with `insert_merge_*` functions.
///
/// The second and final way you may recieve a [`TryFromBoundsError`]
/// is via merging methods such as
/// [`RangeBoundsMap::insert_merge_touching`].
///
/// In the first example it was fairly easy to create an invalid
/// `RangeBounds` by cutting with a different `RangeBounds` than the
/// underlying `RangeBoundsMap`'s `RangeBounds` type. However, the
/// `insert_merge_*` functions all take `range: K` as an
/// argument so it is not possible to create an invalid `K` type
/// directly. However upon "merging" of two ranges (even if
/// both of them are type `K`), you can create a range that *cannot* be
/// of type `K`.
///
/// In this example we use a `RangeBounds` type that can be either
/// Inclusive-Inclusive OR Exclusive-Exclusive. We then try to use
/// [`RangeBoundsMap::insert_merge_touching()`] to "merge" an
/// Inclusive-Inclusive and a Exclusive-Exclusive `MultiBounds`. This
/// will however fail as the resulting "merged" `RangeBounds` would
/// have to be Inclusive-Exclusive which `MultiBounds` does not support.
///
/// ```
/// use std::ops::{Bound, RangeBounds};
///
/// use range_bounds_map::{
/// 	OverlapOrTryFromBoundsError, RangeBoundsMap, TryFromBounds,
/// 	TryFromBoundsError,
/// };
///
/// #[derive(Debug, Copy, Clone, PartialEq)]
/// enum MultiBounds {
/// 	Inclusive(i8, i8),
/// 	Exclusive(i8, i8),
/// }
///
/// impl RangeBounds<i8> for MultiBounds {
/// 	fn start_bound(&self) -> Bound<&i8> {
/// 		match self {
/// 			MultiBounds::Inclusive(start, _) => {
/// 				Bound::Included(start)
/// 			}
/// 			MultiBounds::Exclusive(start, _) => {
/// 				Bound::Excluded(start)
/// 			}
/// 		}
/// 	}
/// 	fn end_bound(&self) -> Bound<&i8> {
/// 		match self {
/// 			MultiBounds::Inclusive(_, end) => {
/// 				Bound::Included(end)
/// 			}
/// 			MultiBounds::Exclusive(_, end) => {
/// 				Bound::Excluded(end)
/// 			}
/// 		}
/// 	}
/// }
///
/// impl TryFromBounds<i8> for MultiBounds {
/// 	fn try_from_bounds(
/// 		start_bound: Bound<i8>,
/// 		end_bound: Bound<i8>,
/// 	) -> Result<Self, TryFromBoundsError> {
/// 		match (start_bound, end_bound) {
/// 			(Bound::Included(start), Bound::Included(end)) => {
/// 				Ok(MultiBounds::Inclusive(start, end))
/// 			}
/// 			(Bound::Excluded(start), Bound::Excluded(end)) => {
/// 				Ok(MultiBounds::Exclusive(start, end))
/// 			}
/// 			_ => Err(TryFromBoundsError),
/// 		}
/// 	}
/// }
///
/// let mut map = RangeBoundsMap::from_slice_strict([(
/// 	MultiBounds::Inclusive(2, 4),
/// 	true,
/// )])
/// .unwrap();
///
/// assert_eq!(
/// 	map.insert_merge_touching(
/// 		MultiBounds::Exclusive(4, 6),
/// 		false
/// 	),
/// 	Err(OverlapOrTryFromBoundsError::TryFromBounds(
/// 		TryFromBoundsError
/// 	))
/// );
/// ```
#[derive(PartialEq, Debug)]
pub struct TryFromBoundsError;

/// An error type to represent either an [`OverlapError`] or a
/// [`TryFromBoundsError`].
#[derive(PartialEq, Debug)]
pub enum OverlapOrTryFromBoundsError {
	Overlap(OverlapError),
	TryFromBounds(TryFromBoundsError),
}

impl<I, K, V> RangeBoundsMap<I, K, V>
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	/// Makes a new, empty `RangeBoundsMap`.
	///
	/// # Examples
	/// ```
	/// use std::ops::Range;
	///
	/// use range_bounds_map::test_ranges::AnyRange;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map: RangeBoundsMap<i8, AnyRange, bool> =
	/// 	RangeBoundsMap::new();
	/// ```
	pub fn new() -> Self {
		RangeBoundsMap {
			inner: BTreeMap::new(),
			phantom: PhantomData,
		}
	}

	/// Returns the number of ranges in the map.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::new();
	///
	/// assert_eq!(map.len(), 0);
	/// map.insert_strict(ie(0, 1), false).unwrap();
	/// assert_eq!(map.len(), 1);
	/// ```
	pub fn len(&self) -> usize {
		self.inner.len()
	}

	/// Returns `true` if the map contains no ranges, and
	/// `false` if it does.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::new();
	///
	/// assert_eq!(map.is_empty(), true);
	/// map.insert_strict(ie(0, 1), false).unwrap();
	/// assert_eq!(map.is_empty(), false);
	/// ```
	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}

	/// Returns `true` if the given range overlaps any of the
	/// other ranges in the map, and `false` if not.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::{ie, ii};
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::new();
	///
	/// map.insert_strict(ie(5, 10), false);
	///
	/// assert_eq!(map.overlaps(ii(1, 3)), false);
	/// assert_eq!(map.overlaps(ie(4, 5)), false);
	///
	/// assert_eq!(map.overlaps(ii(4, 5)), true);
	/// assert_eq!(map.overlaps(ie(4, 6)), true);
	/// ```
	pub fn overlaps<Q>(&self, range: Q) -> bool
	where
		Q: NiceRange<I>,
	{
		invalid_range_panic(range);

		self.overlapping(range).next().is_some()
	}

	/// Returns an iterator over every entry in the map that overlaps
	/// the given range in ascending order.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// let mut overlapping = map.overlapping(ie(2, 8));
	///
	/// assert_eq!(
	/// 	overlapping.collect::<Vec<_>>(),
	/// 	[(&ie(1, 4), &false), (&ie(4, 8), &true)]
	/// );
	/// ```
	pub fn overlapping<Q>(
		&self,
		range: Q,
	) -> impl DoubleEndedIterator<Item = (&K, &V)>
	where
		Q: NiceRange<I>,
	{
		invalid_range_panic(range);

		let start_comp = overlapping_start_comp(range.start());
		let end_comp = overlapping_end_comp(range.end());

		let start_bound = SearchBoundCustom::Included;
		let end_bound = SearchBoundCustom::Included;

		self.inner
			.range(start_comp, start_bound, end_comp, end_bound)
	}

	/// Returns an mutable iterator over every entry in the map that
	/// overlaps the given range in ascending order.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// for (range, value) in map.overlapping_mut(ie(3, 7)) {
	/// 	if *range == ie(4, 8) {
	/// 		*value = false
	/// 	} else {
	/// 		*value = true
	/// 	}
	/// }
	/// ```
	pub fn overlapping_mut<Q>(
		&mut self,
		range: Q,
	) -> impl DoubleEndedIterator<Item = (&K, &mut V)>
	where
		Q: NiceRange<I>,
	{
		invalid_range_panic(range);

		let start_comp = overlapping_start_comp(range.start());
		let end_comp = overlapping_end_comp(range.end());

		let start_bound = SearchBoundCustom::Included;
		let end_bound = SearchBoundCustom::Included;

		self.inner
			.range_mut(start_comp, start_bound, end_comp, end_bound)
	}

	/// Returns a reference to the value corresponding to the range in
	/// the map that overlaps the given point, if any.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(map.get_at_point(3), Some(&false));
	/// assert_eq!(map.get_at_point(4), Some(&true));
	/// assert_eq!(map.get_at_point(101), None);
	/// ```
	pub fn get_at_point(&self, point: I) -> Option<&V> {
		self.get_entry_at_point(point).map(|(_, value)| value).ok()
	}

	/// Returns a mutable reference to the value corresponding to the
	/// range that overlaps the given point, if any.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	/// let mut map =
	/// 	RangeBoundsMap::from_slice_strict([(ie(1, 4), false)])
	/// 		.unwrap();
	///
	/// if let Some(x) = map.get_at_point_mut(2) {
	/// 	*x = true;
	/// }
	///
	/// assert_eq!(map.get_at_point(1), Some(&true));
	/// ```
	pub fn get_at_point_mut(&mut self, point: I) -> Option<&mut V> {
		self.inner
			.get_mut(overlapping_start_comp(Bound::Included(point)))
	}

	/// Returns `true` if the map contains a range that overlaps the
	/// given point, and `false` if not.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(map.contains_point(3), true);
	/// assert_eq!(map.contains_point(4), true);
	/// assert_eq!(map.contains_point(101), false);
	/// ```
	pub fn contains_point(&self, point: I) -> bool {
		self.get_entry_at_point(point).is_ok()
	}

	/// Returns the entry corresponding to the range that
	/// overlaps the given point, if any.
	///
	/// If there is no range that overlaps the given point the
	/// maximally-sized gap at the given point is returned.
	///
	/// # Examples
	/// ```
	/// use std::ops::Bound;
	///
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 6), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(map.get_entry_at_point(3), Ok((&ie(1, 4), &false)));
	/// assert_eq!(map.get_entry_at_point(5), Ok((&ie(4, 6), &true)));
	/// assert_eq!(
	/// 	map.get_entry_at_point(7),
	/// 	Err((Bound::Included(6), Bound::Excluded(8)))
	/// );
	/// assert_eq!(
	/// 	map.get_entry_at_point(101),
	/// 	Err((Bound::Included(100), Bound::Unbounded))
	/// );
	/// ```
	pub fn get_entry_at_point(
		&self,
		point: I,
	) -> Result<(&K, &V), (Bound<I>, Bound<I>)> {
		self.inner
			.get_key_value(overlapping_start_comp(Bound::Included(point)))
			.ok_or_else(|| {
				let lower = self.inner.upper_bound(
					overlapping_start_comp(Bound::Included(point)),
					SearchBoundCustom::Included,
				);
				let upper = self.inner.lower_bound(
					overlapping_end_comp(Bound::Included(point)),
					SearchBoundCustom::Included,
				);

				(
					lower.key().map_or(Bound::Unbounded, |lower| {
						flip_bound(lower.end())
					}),
					upper.key().map_or(Bound::Unbounded, |upper| {
						flip_bound(upper.start())
					}),
				)
			})
	}

	/// Returns an iterator over every entry in the map in ascending
	/// order.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// let mut iter = map.iter();
	///
	/// assert_eq!(iter.next(), Some((&ie(1, 4), &false)));
	/// assert_eq!(iter.next(), Some((&ie(4, 8), &true)));
	/// assert_eq!(iter.next(), Some((&ie(8, 100), &false)));
	/// assert_eq!(iter.next(), None);
	/// ```
	pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&K, &V)> {
		self.inner.iter()
	}

	/// Returns an mutable iterator over every entry in the map in
	/// ascending order.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// for (range, value) in map.iter_mut() {
	/// 	if *range == ie(4, 8) {
	/// 		*value = false
	/// 	} else {
	/// 		*value = true
	/// 	}
	/// }
	/// ```
	pub fn iter_mut(
		&mut self,
	) -> impl DoubleEndedIterator<Item = (&K, &mut V)> {
		self.inner.iter_mut()
	}

	/// Removes every entry in the map which overlaps the given range
	/// and returns them in an iterator.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// let mut removed = map.remove_overlapping(ie(2, 8));
	///
	/// assert_eq!(
	/// 	removed.collect::<Vec<_>>(),
	/// 	[(ie(1, 4), false), (ie(4, 8), true)]
	/// );
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(8, 100), false)]
	/// );
	/// ```
	pub fn remove_overlapping<'a, Q>(
		&'a mut self,
		range: Q,
	) -> impl Iterator<Item = (K, V)> + '_
	where
		Q: NiceRange<I> + 'a,
	{
		invalid_range_panic(range);

		//optimisation, switch to BTreeMap::drain_range if it ever gets
		//implemented
		return self
			.inner
			.drain_filter(move |inner_range, _| overlaps(*inner_range, range));
	}

	/// Cuts a given range out of the map and returns an iterator of
	/// the full or partial ranges that were cut.
	///
	/// If the remaining ranges left in the map after the cut would
	/// not be able be created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned and the map will not
	/// be cut at all.
	///
	/// `V` must implement `Clone` as if you try to cut out the center
	/// of a range in the map it will split into two different entries
	/// using `Clone`. Or if you partially cut a range then
	/// `V` must be cloned to be returned in the iterator.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use std::ops::Bound;
	///
	/// use range_bounds_map::test_ranges::{ie, ie_strict, ii};
	/// use range_bounds_map::{RangeBoundsMap, TryFromBoundsError};
	///
	/// let mut base = RangeBoundsMap::from_slice_strict([
	/// 	(ie_strict(1, 4), false),
	/// 	(ie_strict(4, 8), true),
	/// 	(ie_strict(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// let after_cut = RangeBoundsMap::from_slice_strict([
	/// 	(ie_strict(1, 2), false),
	/// 	(ie_strict(40, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(
	/// 	base.cut(ie(2, 40)).unwrap().collect::<Vec<_>>(),
	/// 	[
	/// 		((Bound::Included(2), Bound::Excluded(4)), false),
	/// 		((Bound::Included(4), Bound::Excluded(8)), true),
	/// 		((Bound::Included(8), Bound::Excluded(40)), false),
	/// 	]
	/// );
	/// assert_eq!(base, after_cut);
	/// assert!(base.cut(ii(60, 80)).is_err());
	/// ```
	pub fn cut<'a, Q>(
		&'a mut self,
		range: Q,
	) -> Result<
		impl Iterator<Item = ((Bound<I>, Bound<I>), V)> + '_,
		TryFromBoundsError,
	>
	where
		Q: NiceRange<I> + 'a,
		K: TryFrom<DiscreteBounds<I>>,
		V: Clone,
	{
		invalid_range_panic(range);

		let start_comp = overlapping_start_comp(range.start());
		let end_comp = overlapping_end_comp(range.end());

		let left_overlapping = self
			.inner
			.get_key_value(start_comp)
			.map(|(key, _)| key)
			.copied();
		let right_overlapping = self
			.inner
			.get_key_value(end_comp)
			.map(|(key, _)| key)
			.copied();

		if let Some(left) = left_overlapping && let Some(right) = right_overlapping && left.start() == right.start() {
            Ok(Either::Left(self.cut_single_overlapping(range, left)?))
        } else {
            Ok(Either::Right(self.cut_non_single_overlapping(range, left_overlapping, right_overlapping)?))
        }
	}
	fn cut_single_overlapping<Q>(
		&mut self,
		range: Q,
		single_overlapping_range: K,
	) -> Result<
		impl Iterator<Item = ((Bound<I>, Bound<I>), V)>,
		TryFromBoundsError,
	>
	where
		Q: NiceRange<I>,
		K: TryFrom<DiscreteBounds<I>>,
		V: Clone,
	{
		invalid_range_panic(range);

		let cut_result = cut_range(single_overlapping_range, range);
		let returning_before_cut = match cut_result.before_cut {
			Some((start, end)) => Some(K::try_from_bounds(start, end)?),
			None => None,
		};
		let returning_after_cut = match cut_result.after_cut {
			Some((start, end)) => Some(K::try_from_bounds(start, end)?),
			None => None,
		};

		let value = self
			.inner
			.remove(overlapping_start_comp(range.start()))
			.unwrap();

		if let Some(before) = returning_before_cut {
			self.insert_unchecked(before, value.clone());
		}
		if let Some(after) = returning_after_cut {
			self.insert_unchecked(after, value.clone());
		}

		Ok(once((cut_result.inside_cut.unwrap(), value)))
	}
	fn cut_non_single_overlapping<'a, Q>(
		&'a mut self,
		range: Q,
		left_overlapping: Option<K>,
		right_overlapping: Option<K>,
	) -> Result<
		impl Iterator<Item = ((Bound<I>, Bound<I>), V)> + '_,
		TryFromBoundsError,
	>
	where
		Q: NiceRange<I> + 'a,
		K: TryFrom<DiscreteBounds<I>>,
		V: Clone,
	{
		invalid_range_panic(range);

		let before_config = match left_overlapping {
			Some(before) => {
				let cut_result = cut_range(before, range);

				Some((
					match cut_result.before_cut {
						Some((start, end)) => {
							Some(K::try_from_bounds(start, end)?)
						}
						None => None,
					},
					cut_result.inside_cut.unwrap(),
				))
			}
			None => None,
		};
		let after_config = match right_overlapping {
			Some(after) => {
				let cut_result = cut_range(after, range);

				Some((
					match cut_result.after_cut {
						Some((start, end)) => {
							Some(K::try_from_bounds(start, end)?)
						}
						None => None,
					},
					cut_result.inside_cut.unwrap(),
				))
			}
			None => None,
		};

		let before_value =
			self.inner.remove(overlapping_start_comp(range.start()));
		let after_value = self.inner.remove(overlapping_end_comp(range.end()));

		if let Some((Some(returning_before_cut), _)) = before_config {
			self.insert_unchecked(
				returning_before_cut,
				before_value.as_ref().cloned().unwrap(),
			);
		}
		if let Some((Some(returning_after_cut), _)) = after_config {
			self.insert_unchecked(
				returning_after_cut,
				after_value.as_ref().cloned().unwrap(),
			);
		}

		let keeping_before_entry =
			before_config.map(|(_, keeping_before_entry)| {
				(keeping_before_entry, before_value.unwrap())
			});
		let keeping_after_entry =
			after_config.map(|(_, keeping_after_entry)| {
				(keeping_after_entry, after_value.unwrap())
			});

		return Ok(keeping_before_entry
			.into_iter()
			.chain(
				self.remove_overlapping(range)
					.map(|(key, value)| ((key.start(), key.end()), value)),
			)
			.chain(keeping_after_entry.into_iter()));
	}

	/// Returns an iterator of ranges over all the maximally-sized
	/// gaps in the map that are also within the given `outer_range`.
	///
	/// To get all possible gaps call `gaps()` with an unbounded range
	/// such as `(Bound::Unbounded, Bound::Unbounded)`.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use std::ops::Bound;
	///
	/// use range_bounds_map::test_ranges::{ie, iu};
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 3), false),
	/// 	(ie(5, 7), true),
	/// 	(ie(9, 100), false),
	/// ])
	/// .unwrap();
	///
	/// let mut gaps = map.gaps(iu(2));
	///
	/// assert_eq!(
	/// 	gaps.collect::<Vec<_>>(),
	/// 	[
	/// 		(Bound::Included(3), Bound::Excluded(5)),
	/// 		(Bound::Included(7), Bound::Excluded(9)),
	/// 		(Bound::Included(100), Bound::Unbounded)
	/// 	]
	/// );
	/// ```
	pub fn gaps<Q>(
		&self,
		outer_range: Q,
	) -> impl DoubleEndedIterator<Item = (Bound<I>, Bound<I>)>
	where
		Q: NiceRange<I>,
	{
		invalid_range_panic(outer_range);

		// I'm in love with how clean/mindblowing this entire function is
		let overlapping = self
			.overlapping(outer_range)
			.map(|(key, _)| (key.start(), key.end()));

		// If the start or end point of outer_range is not
		// contained within a RangeBounds in the map then we need to
		// generate a artificial RangeBounds to use instead.
		//
		// We also have to flip the artificial ones ahead of time as
		// we actually want the range endpoints included
		// not excluded unlike with other bounds in artificials

		let artificial_start = (
			flip_bound(outer_range.start()),
			flip_bound(outer_range.start()),
		);
		let artificial_end =
			(flip_bound(outer_range.end()), flip_bound(outer_range.end()));
		let mut artificials = once(artificial_start)
			.chain(overlapping)
			.chain(once(artificial_end));

		let start_contained = self
			.inner
			.contains_key(overlapping_start_comp(outer_range.start()));
		let end_contained = self
			.inner
			.contains_key(overlapping_end_comp(outer_range.end()));

		if start_contained {
			artificials.next();
		}
		if end_contained {
			artificials.next_back();
		}

		return artificials
			//optimisation find an implementation of windows()
			//somewhere that supports DoubleEndedIterator, I couldn't
			//find one at the time of writing
			.collect::<Vec<_>>()
			.windows(2)
			.map(|windows| (flip_bound(windows[0].1), flip_bound(windows[1].0)))
			.filter(|range| is_valid_range(*range))
			//optimisation this would also then be unneccessary
			.collect::<Vec<_>>()
			.into_iter();
	}

	/// Returns `true` if the map covers every point in the given
	/// range, and `false` if it does not.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 3), false),
	/// 	(ie(5, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(map.contains_range(ie(1, 3)), true);
	/// assert_eq!(map.contains_range(ie(2, 6)), false);
	/// assert_eq!(map.contains_range(ie(6, 100)), true);
	/// ```
	pub fn contains_range<Q>(&self, range: Q) -> bool
	where
		Q: NiceRange<I>,
	{
		invalid_range_panic(range);

		// Soooo clean and mathematical 🥰!
		self.gaps(range).next().is_none()
	}

	/// Adds a new entry to the map without modifying other entries.
	///
	/// If the given range overlaps one or more ranges already in the
	/// map, then an [`OverlapError`] is returned and the map is not
	/// updated.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{OverlapError, RangeBoundsMap};
	///
	/// let mut map = RangeBoundsMap::new();
	///
	/// assert_eq!(map.insert_strict(ie(5, 10), 9), Ok(()));
	/// assert_eq!(map.insert_strict(ie(5, 10), 2), Err(OverlapError));
	/// assert_eq!(map.len(), 1);
	/// ```
	pub fn insert_strict(
		&mut self,
		range: K,
		value: V,
	) -> Result<(), OverlapError> {
		invalid_range_panic(range);

		if self.overlaps(range) {
			return Err(OverlapError);
		}

		self.insert_unchecked(range, value);

		return Ok(());
	}
	fn insert_unchecked(&mut self, range: K, value: V) {
		self.inner.insert(range, value, double_comp());
	}

	fn insert_merge_with_comps<G1, G2, R1, R2>(
		&mut self,
		range: K,
		value: V,
		get_start: G1,
		get_end: G2,
		remove_start: R1,
		remove_end: R2,
	) -> Result<K, TryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
		G1: FnOnce(&Self, &V) -> Option<K>,
		G2: FnOnce(&Self, &V) -> Option<K>,
		R1: FnOnce(&mut Self, &V),
		R2: FnOnce(&mut Self, &V),
	{
		invalid_range_panic(range);

		let matching_start = get_start(self, &value);
		let matching_end = get_end(self, &value);

		let returning = match (matching_start, matching_end) {
			(Some(matching_start), Some(matching_end)) => {
				K::try_from_bounds(matching_start.start(), matching_end.end())?
			}
			(Some(matching_start), None) => {
				K::try_from_bounds(matching_start.start(), range.end())?
			}
			(None, Some(matching_end)) => {
				K::try_from_bounds(range.start(), matching_end.end())?
			}
			(None, None) => range,
		};

		let _ = self.remove_overlapping(range);

		remove_start(self, &value);
		remove_end(self, &value);

		self.insert_unchecked(returning, value);

		Ok(returning)
	}

	/// Adds a new entry to the map and merges into other ranges in
	/// the map which touch it.
	///
	/// The value of the merged-together range is set to the value given for
	/// this insertion.
	///
	/// If successful then the newly inserted (possibly merged) range is
	/// returned.
	///
	/// If the given range overlaps one or more ranges already in the
	/// map, then an [`OverlapError`] is returned and the map is not
	/// updated.
	///
	/// If the range merges with one or two touching ranges and the
	/// merged-together range cannot be created with the
	/// [`TryFromBounds`] trait then a [`TryFromBoundsError`] will be
	/// returned.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{
	/// 	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap,
	/// };
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(6, 8), true),
	/// ])
	/// .unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	map.insert_merge_touching(ie(4, 6), true),
	/// 	Ok(ie(1, 8))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching(ie(4, 8), false),
	/// 	Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching(ie(10, 16), false),
	/// 	Ok(ie(10, 16))
	/// );
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(1, 8), true), (ie(10, 16), false)]
	/// );
	/// ```
	pub fn insert_merge_touching(
		&mut self,
		range: K,
		value: V,
	) -> Result<K, OverlapOrTryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
	{
		invalid_range_panic(range);

		if self.overlaps(range) {
			return Err(OverlapOrTryFromBoundsError::Overlap(OverlapError));
		}

		self.insert_merge_with_comps(
			range,
			value,
			|selfy, _| {
				selfy
					.inner
					.get_key_value(touching_start_comp(range.start()))
					.map(|(key, _)| key)
					.copied()
			},
			|selfy, _| {
				selfy
					.inner
					.get_key_value(touching_end_comp(range.end()))
					.map(|(key, _)| key)
					.copied()
			},
			|selfy, _| {
				selfy.inner.remove(touching_start_comp(range.start()));
			},
			|selfy, _| {
				selfy.inner.remove(touching_end_comp(range.end()));
			},
		)
		.map_err(OverlapOrTryFromBoundsError::TryFromBounds)
	}

	/// Adds a new entry to the map and merges into other ranges in
	/// the map which touch it if the touching ranges' values are
	/// equal to the value being inserted.
	///
	/// If successful then the newly inserted (possibly merged) range is
	/// returned.
	///
	/// If the given range overlaps one or more ranges already in the
	/// map, then an [`OverlapError`] is returned and the map is not
	/// updated.
	///
	/// If the range merges with one or two touching ranges and the
	/// merged-together range cannot be created with the
	/// [`TryFromBounds`] trait then a [`TryFromBoundsError`] will be
	/// returned.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{
	/// 	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap,
	/// };
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(6, 8), true),
	/// ])
	/// .unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	map.insert_merge_touching_if_values_equal(ie(4, 6), true),
	/// 	Ok(ie(4, 8))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching_if_values_equal(ie(4, 8), false),
	/// 	Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching_if_values_equal(ie(10, 16), false),
	/// 	Ok(ie(10, 16))
	/// );
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(1, 4), false), (ie(4, 8), true), (ie(10, 16), false)]
	/// );
	/// ```
	pub fn insert_merge_touching_if_values_equal(
		&mut self,
		range: K,
		value: V,
	) -> Result<K, OverlapOrTryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
		V: Eq,
	{
		invalid_range_panic(range);

		if self.overlaps(range) {
			return Err(OverlapOrTryFromBoundsError::Overlap(OverlapError));
		}

		let get_start = |selfy: &Self, value: &V| {
			selfy
				.inner
				.get_key_value(touching_start_comp(range.start()))
				.filter(|(_, start_touching_value)| {
					*start_touching_value == value
				})
				.map(|(key, _)| key)
				.copied()
		};
		let get_end = |selfy: &Self, value: &V| {
			selfy
				.inner
				.get_key_value(touching_end_comp(range.end()))
				.filter(|(_, start_touching_value)| {
					*start_touching_value == value
				})
				.map(|(key, _)| key)
				.copied()
		};

		self.insert_merge_with_comps(
			range,
			value,
			get_start,
			get_end,
			|selfy, value| {
				if get_start(selfy, value).is_some() {
					selfy.inner.remove(touching_start_comp(range.start()));
				}
			},
			|selfy, value| {
				if get_end(selfy, value).is_some() {
					selfy.inner.remove(touching_end_comp(range.end()));
				}
			},
		)
		.map_err(OverlapOrTryFromBoundsError::TryFromBounds)
	}

	/// Adds a new entry to the map and merges into other ranges in
	/// the map which overlap it.
	///
	/// The value of the merged-together range is set to the value given for
	/// this insertion.
	///
	/// If successful then the newly inserted (possibly merged) range is
	/// returned.
	///
	/// If the range merges other ranges and the merged-together range
	/// cannot be created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{
	/// 	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap,
	/// };
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(6, 8), true),
	/// ])
	/// .unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	map.insert_merge_overlapping(ie(4, 6), true),
	/// 	Ok(ie(4, 6))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_overlapping(ie(4, 8), false),
	/// 	Ok(ie(4, 8))
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_overlapping(ie(10, 16), false),
	/// 	Ok(ie(10, 16))
	/// );
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(1, 4), false), (ie(4, 8), false), (ie(10, 16), false)]
	/// );
	/// ```
	pub fn insert_merge_overlapping(
		&mut self,
		range: K,
		value: V,
	) -> Result<K, TryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
	{
		invalid_range_panic(range);

		self.insert_merge_with_comps(
			range,
			value,
			|selfy, _| {
				selfy
					.inner
					.get_key_value(overlapping_start_comp(range.start()))
					.map(|(key, _)| key)
					.copied()
			},
			|selfy, _| {
				selfy
					.inner
					.get_key_value(overlapping_end_comp(range.end()))
					.map(|(key, _)| key)
					.copied()
			},
			|_, _| {},
			|_, _| {},
		)
	}

	/// Adds a new entry to the map and merges into other ranges in
	/// the map which touch or overlap it.
	///
	/// The value of the merged-together range is set to the value given for
	/// this insertion.
	///
	/// If successful then the newly inserted (possibly merged) range is
	/// returned.
	///
	/// If the range merges other ranges and the merged-together range
	/// cannot be created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{
	/// 	OverlapError, OverlapOrTryFromBoundsError, RangeBoundsMap,
	/// };
	///
	/// let mut map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(6, 8), true),
	/// ])
	/// .unwrap();
	///
	/// // Touching
	/// assert_eq!(
	/// 	map.insert_merge_touching_or_overlapping(ie(4, 6), true),
	/// 	Ok(ie(1, 8))
	/// );
	///
	/// // Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching_or_overlapping(ie(4, 8), false),
	/// 	Ok(ie(1, 8))
	/// );
	///
	/// // Neither Touching or Overlapping
	/// assert_eq!(
	/// 	map.insert_merge_touching_or_overlapping(ie(10, 16), false),
	/// 	Ok(ie(10, 16))
	/// );
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(1, 8), false), (ie(10, 16), false)]
	/// );
	/// ```
	pub fn insert_merge_touching_or_overlapping(
		&mut self,
		range: K,
		value: V,
	) -> Result<K, TryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
	{
		invalid_range_panic(range);

		self.insert_merge_with_comps(
			range,
			value,
			|selfy, _| {
				selfy
					.inner
					.get_key_value(touching_start_comp(range.start()))
					.map(|(key, _)| key)
					.or(selfy
						.inner
						.get_key_value(overlapping_start_comp(range.start()))
						.map(|(key, _)| key))
					.copied()
			},
			|selfy, _| {
				selfy
					.inner
					.get_key_value(touching_end_comp(range.end()))
					.map(|(key, _)| key)
					.or(selfy
						.inner
						.get_key_value(overlapping_end_comp(range.end()))
						.map(|(key, _)| key))
					.copied()
			},
			|selfy, _| {
				selfy.inner.remove(touching_start_comp(range.start()));
			},
			|selfy, _| {
				selfy.inner.remove(touching_end_comp(range.end()));
			},
		)
	}

	/// Adds a new entry to the map and overwrites any other ranges
	/// that overlap the new range.
	///
	/// This is equivalent to using [`RangeBoundsMap::cut()`]
	/// followed by [`RangeBoundsMap::insert_strict()`]. Hence the
	/// same `V: Clone` trait bound applies.
	///
	/// If the remaining ranges left after the cut are not able to be
	/// created with the [`TryFromBounds`] trait then a
	/// [`TryFromBoundsError`] will be returned.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let mut map =
	/// 	RangeBoundsMap::from_slice_strict([(ie(2, 8), false)])
	/// 		.unwrap();
	///
	/// assert_eq!(map.insert_overwrite(ie(4, 6), true), Ok(()));
	///
	/// assert_eq!(
	/// 	map.into_iter().collect::<Vec<_>>(),
	/// 	[(ie(2, 4), false), (ie(4, 6), true), (ie(6, 8), false)]
	/// );
	/// ```
	pub fn insert_overwrite(
		&mut self,
		range: K,
		value: V,
	) -> Result<(), TryFromBoundsError>
	where
		K: TryFrom<DiscreteBounds<I>>,
		V: Clone,
	{
		invalid_range_panic(range);

		let _ = self.cut(range)?;
		self.insert_unchecked(range, value);

		return Ok(());
	}

	/// Returns the first entry in the map, if any.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::RangeBoundsMap;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(map.first_entry(), Some((&ie(1, 4), &false)));
	/// ```
	pub fn first_entry(&self) -> Option<(&K, &V)> {
		self.inner.first_key_value()
	}

	/// Returns the last entry in the map, if any.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::RangeBoundsMap;
	/// use range_bounds_map::test_ranges::ie;
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	///
	/// assert_eq!(
	/// 	map.last_entry(),
	/// 	Some((&ie(8, 100), &false))
	/// );
	pub fn last_entry(&self) -> Option<(&K, &V)> {
		self.inner.last_key_value()
	}

	/// Allocates a `RangeBoundsMap` and moves the given entries from
	/// the given slice into the map using
	/// [`RangeBoundsMap::insert_strict()`].
	///
	/// May return an `Err` while inserting. See
	/// [`RangeBoundsMap::insert_strict()`] for details.
	///
	/// # Panics
	///
	/// Panics if the given range is an invalid range. See [`Invalid
	/// Ranges`](https://docs.rs/range_bounds_map/latest/range_bounds_map/index.html#invalid-ranges)
	/// for more details.
	///
	/// # Examples
	/// ```
	/// use range_bounds_map::test_ranges::ie;
	/// use range_bounds_map::{RangeBoundsMap, TryFromBoundsError};
	///
	/// let map = RangeBoundsMap::from_slice_strict([
	/// 	(ie(1, 4), false),
	/// 	(ie(4, 8), true),
	/// 	(ie(8, 100), false),
	/// ])
	/// .unwrap();
	/// ```
	pub fn from_slice_strict<const N: usize>(
		slice: [(K, V); N],
	) -> Result<RangeBoundsMap<I, K, V>, OverlapError> {
		let mut map = RangeBoundsMap::new();
		for (range, value) in slice {
			map.insert_strict(range, value)?;
		}
		return Ok(map);
	}
}

// Helper Functions ==========================

fn invalid_range_panic<Q, I>(range: Q)
where
	Q: NiceRange<I>,
	I: Ord,
{
	if !is_valid_range(range) {
		panic!(
			"invalid range given to function see here for more details: https://docs.rs/range_bounds_map/latest/range_bounds_map/#invalid-ranges"
		);
	}
}

fn double_comp<K, I>() -> impl FnMut(&K, &K) -> Ordering
where
	K: NiceRange<I>,
	I: Ord,
{
	|inner_range: &K, new_range: &K| {
		DiscreteBoundOrd::start(new_range.start())
			.cmp(&DiscreteBoundOrd::start(inner_range.start()))
	}
}
fn overlapping_start_comp<I, K>(start: Bound<I>) -> impl FnMut(&K) -> Ordering
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	move |inner_range: &K| {
		cmp_range_with_discrete_bound_ord(
			*inner_range,
			DiscreteBoundOrd::start(start),
		)
	}
}
fn overlapping_end_comp<I, K>(end: Bound<I>) -> impl FnMut(&K) -> Ordering
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	move |inner_range: &K| {
		cmp_range_with_discrete_bound_ord(
			*inner_range,
			DiscreteBoundOrd::end(end),
		)
	}
}
fn touching_start_comp<I, K>(start: Bound<I>) -> impl FnMut(&K) -> Ordering
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	move |inner_range: &K| match (inner_range.end(), start) {
		//we only allow Ordering::Equal here since if they are equal
		//then the ranges would be touching
		(Bound::Included(end), Bound::Excluded(start)) if end == start => {
			Ordering::Equal
		}
		(Bound::Excluded(end), Bound::Included(start)) if end == start => {
			Ordering::Equal
		}

		(end, start) => {
			let normal_result =
				DiscreteBoundOrd::start(start).cmp(&DiscreteBoundOrd::end(end));

			//we overide any Equals to a random non-Equal since we
			//don't want non-touching matches
			match normal_result {
				Ordering::Equal => Ordering::Greater,
				x => x,
			}
		}
	}
}
fn touching_end_comp<I, K>(end: Bound<I>) -> impl FnMut(&K) -> Ordering
where
	I: Ord + Copy,
	K: NiceRange<I>,
{
	move |inner_range: &K| match (end, inner_range.start()) {
		//we only allow Ordering::Equal here since if they are equal
		//then the ranges would be touching
		(Bound::Included(end), Bound::Excluded(start)) if end == start => {
			Ordering::Equal
		}
		(Bound::Excluded(end), Bound::Included(start)) if end == start => {
			Ordering::Equal
		}

		(end, _start) => {
			let normal_result = DiscreteBoundOrd::end(end)
				.cmp(&DiscreteBoundOrd::start(inner_range.start()));

			//we overide any Equals to a random non-Equal since we
			//don't want non-touching matches
			match normal_result {
				Ordering::Equal => Ordering::Less,
				x => x,
			}
		}
	}
}

/// A simple helper trait to make my implemtation nicer, if you
/// already implement RangeBounds and Copy on your type then this will
/// also be implemted.
pub trait NiceRange<I>: Copy {
	fn start(&self) -> DiscreteBoundOrd<I>;
	fn end(&self) -> DiscreteBoundOrd<I>;
}
impl<K, I> NiceRange<I> for K
where
	I: Copy,
	K: RangeBounds<I> + Copy,
{
	fn start(&self) -> Bound<I> {
		self.start_bound().cloned()
	}
	fn end(&self) -> Bound<I> {
		self.end_bound().cloned()
	}
}

// Trait Impls ==========================

impl<I, K, V> IntoIterator for RangeBoundsMap<I, K, V> {
	type Item = (K, V);
	type IntoIter = IntoIter<I, K, V>;
	fn into_iter(self) -> Self::IntoIter {
		return IntoIter {
			inner: self.inner.into_iter(),
			phantom: PhantomData,
		};
	}
}
/// An owning iterator over the entries of a [`RangeBoundsMap`].
///
/// This `struct` is created by the [`into_iter`] method on
/// [`RangeBoundsMap`] (provided by the [`IntoIterator`] trait). See
/// its documentation for more.
///
/// [`into_iter`]: IntoIterator::into_iter
/// [`IntoIterator`]: core::iter::IntoIterator
pub struct IntoIter<I, K, V> {
	inner: BTreeMapIntoIter<K, V>,
	phantom: PhantomData<I>,
}
impl<I, K, V> Iterator for IntoIter<I, K, V> {
	type Item = (K, V);
	fn next(&mut self) -> Option<Self::Item> {
		self.inner.next()
	}
}

impl<I, K, V> Default for RangeBoundsMap<I, K, V> {
	fn default() -> Self {
		RangeBoundsMap {
			inner: BTreeMap::default(),
			phantom: PhantomData,
		}
	}
}

impl<I, K, V> Serialize for RangeBoundsMap<I, K, V>
where
	I: Ord + Copy,
	K: NiceRange<I> + Serialize,
	V: Serialize,
{
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let mut map = serializer.serialize_map(Some(self.len()))?;
		for (range_bounds, value) in self.iter() {
			map.serialize_entry(range_bounds, value)?;
		}
		map.end()
	}
}

impl<'de, I, K, V> Deserialize<'de> for RangeBoundsMap<I, K, V>
where
	I: Ord + Copy,
	K: NiceRange<I> + Deserialize<'de>,
	V: Deserialize<'de>,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		deserializer.deserialize_map(RangeBoundsMapVisitor {
			i: PhantomData,
			k: PhantomData,
			v: PhantomData,
		})
	}
}

struct RangeBoundsMapVisitor<I, K, V> {
	i: PhantomData<I>,
	k: PhantomData<K>,
	v: PhantomData<V>,
}

impl<'de, I, K, V> Visitor<'de> for RangeBoundsMapVisitor<I, K, V>
where
	I: Ord + Copy,
	K: NiceRange<I> + Deserialize<'de>,
	V: Deserialize<'de>,
{
	type Value = RangeBoundsMap<I, K, V>;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a RangeBoundsMap")
	}

	fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
	where
		A: MapAccess<'de>,
	{
		let mut map = RangeBoundsMap::new();
		while let Some((range_bounds, value)) = access.next_entry()? {
			map.insert_strict(range_bounds, value)
				.map_err(|_| serde::de::Error::custom("RangeBounds overlap"))?;
		}
		Ok(map)
	}
}

#[cfg(test)]
mod tests {
	use std::ops::Bound;

	use pretty_assertions::assert_eq;

	use super::*;
	use crate::bound_ord::DiscreteBoundOrd;
	use crate::test_ranges::{ee, ei, ie, ii, iu, u, ue, ui, uu, AnyRange};
	use crate::utils::{config, Config, CutResult};

	//only every other number to allow mathematical_overlapping_definition
	//to test between bounds in finite using smaller intervalled finite
	pub(crate) const NUMBERS: &'static [i8] = &[2, 4, 6, 8, 10];
	//go a bit around on either side to compensate for Unbounded
	pub(crate) const NUMBERS_DOMAIN: &'static [i8] =
		&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

	fn basic() -> RangeBoundsMap<i8, AnyRange, bool> {
		RangeBoundsMap::from_slice_strict([
			(ui(4), false),
			(ee(5, 7), true),
			(ii(7, 7), false),
			(ie(14, 16), true),
		])
		.unwrap()
	}

	fn special() -> RangeBoundsMap<i8, MultiBounds, bool> {
		RangeBoundsMap::from_slice_strict([
			(mii(4, 6), false),
			(mee(7, 8), true),
			(mii(8, 12), false),
		])
		.unwrap()
	}

	#[derive(Debug, PartialEq, Copy, Clone)]
	enum MultiBounds {
		Inclusive(i8, i8),
		Exclusive(i8, i8),
	}

	fn mii(start: i8, end: i8) -> MultiBounds {
		MultiBounds::Inclusive(start, end)
	}
	fn mee(start: i8, end: i8) -> MultiBounds {
		MultiBounds::Exclusive(start, end)
	}

	impl RangeBounds<i8> for MultiBounds {
		fn start_bound(&self) -> Bound<&i8> {
			match self {
				MultiBounds::Inclusive(start, _) => Bound::Included(start),
				MultiBounds::Exclusive(start, _) => Bound::Excluded(start),
			}
		}
		fn end_bound(&self) -> Bound<&i8> {
			match self {
				MultiBounds::Inclusive(_, end) => Bound::Included(end),
				MultiBounds::Exclusive(_, end) => Bound::Excluded(end),
			}
		}
	}

	#[test]
	fn insert_strict_tests() {
		assert_insert_strict(
			basic(),
			(ii(0, 4), false),
			Err(OverlapError),
			None::<[_; 0]>,
		);
		assert_insert_strict(
			basic(),
			(ii(5, 6), false),
			Err(OverlapError),
			None::<[_; 0]>,
		);
		assert_insert_strict(
			basic(),
			(ee(7, 8), false),
			Ok(()),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(7, 8), false),
				(ie(14, 16), true),
			]),
		);
		assert_insert_strict(
			basic(),
			(ii(4, 5), true),
			Err(OverlapError),
			None::<[_; 0]>,
		);
		assert_insert_strict(
			basic(),
			(ei(4, 5), true),
			Ok(()),
			Some([
				(ui(4), false),
				(ei(4, 5), true),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), true),
			]),
		);
	}
	fn assert_insert_strict<const N: usize>(
		mut before: RangeBoundsMap<i8, AnyRange, bool>,
		to_insert: (AnyRange, bool),
		result: Result<(), OverlapError>,
		after: Option<[(AnyRange, bool); N]>,
	) {
		let clone = before.clone();
		assert_eq!(before.insert_strict(to_insert.0, to_insert.1), result);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn overlapping_tests() {
		//case zero
		for overlap_range in all_valid_test_bounds() {
			//you can't overlap nothing
			assert!(
				RangeBoundsMap::<i8, AnyRange, ()>::new()
					.overlapping(overlap_range)
					.next()
					.is_none()
			);
		}

		//case one
		for overlap_range in all_valid_test_bounds() {
			for inside_range in all_valid_test_bounds() {
				let mut map = RangeBoundsMap::new();
				map.insert_strict(inside_range, ()).unwrap();

				let mut expected_overlapping = Vec::new();
				if overlaps(overlap_range, inside_range) {
					expected_overlapping.push(inside_range);
				}

				let overlapping = map
					.overlapping(overlap_range)
					.map(|(key, _)| key)
					.copied()
					.collect::<Vec<_>>();

				if overlapping != expected_overlapping {
					dbg!(overlap_range, inside_range);
					dbg!(overlapping, expected_overlapping);
					panic!(
						"Discrepency in .overlapping() with single inside range detected!"
					);
				}
			}
		}

		//case two
		for overlap_range in all_valid_test_bounds() {
			for (inside_range1, inside_range2) in
				all_non_overlapping_test_bound_entries()
			{
				let mut map = RangeBoundsMap::new();
				map.insert_strict(inside_range1, ()).unwrap();
				map.insert_strict(inside_range2, ()).unwrap();

				let mut expected_overlapping = Vec::new();
				if overlaps(overlap_range, inside_range1) {
					expected_overlapping.push(inside_range1);
				}
				if overlaps(overlap_range, inside_range2) {
					expected_overlapping.push(inside_range2);
				}
				//make our expected_overlapping the correct order
				if expected_overlapping.len() > 1 {
					if DiscreteBoundOrd::start(expected_overlapping[0].start())
						> DiscreteBoundOrd::start(
							expected_overlapping[1].start(),
						) {
						expected_overlapping.swap(0, 1);
					}
				}

				let overlapping = map
					.overlapping(overlap_range)
					.map(|(key, _)| key)
					.copied()
					.collect::<Vec<_>>();

				if overlapping != expected_overlapping {
					dbg!(overlap_range, inside_range1, inside_range2);
					dbg!(overlapping, expected_overlapping);
					panic!(
						"Discrepency in .overlapping() with two inside ranges detected!"
					);
				}
			}
		}
	}

	#[test]
	fn remove_overlapping_tests() {
		assert_remove_overlapping(basic(), ii(5, 5), [], None::<[_; 0]>);
		assert_remove_overlapping(
			basic(),
			uu(),
			[
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), true),
			],
			Some([]),
		);
		assert_remove_overlapping(
			basic(),
			ii(6, 7),
			[(ee(5, 7), true), (ii(7, 7), false)],
			Some([(ui(4), false), (ie(14, 16), true)]),
		);
		assert_remove_overlapping(
			basic(),
			iu(6),
			[(ee(5, 7), true), (ii(7, 7), false), (ie(14, 16), true)],
			Some([(ui(4), false)]),
		);
	}
	fn assert_remove_overlapping<const N: usize, const Y: usize>(
		mut before: RangeBoundsMap<i8, AnyRange, bool>,
		to_remove: AnyRange,
		result: [(AnyRange, bool); N],
		after: Option<[(AnyRange, bool); Y]>,
	) {
		let clone = before.clone();
		assert_eq!(
			before.remove_overlapping(to_remove).collect::<Vec<_>>(),
			result
		);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn cut_tests() {
		assert_cut(basic(), ii(50, 60), Ok([]), None::<[_; 0]>);
		assert_cut(
			basic(),
			uu(),
			Ok([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), true),
			]),
			Some([]),
		);
		assert_cut(
			basic(),
			ui(6),
			Ok([(ui(4), false), (ei(5, 6), true)]),
			Some([(ee(6, 7), true), (ii(7, 7), false), (ie(14, 16), true)]),
		);
		assert_cut(
			basic(),
			iu(6),
			Ok([(ie(6, 7), true), (ii(7, 7), false), (ie(14, 16), true)]),
			Some([(ui(4), false), (ee(5, 6), true)]),
		);

		assert_cut(
			special(),
			mee(5, 7),
			Ok([((Bound::Excluded(5), Bound::Included(6)), false)]),
			Some([(mii(4, 5), false), (mee(7, 8), true), (mii(8, 12), false)]),
		);
		assert_cut(special(), mee(6, 7), Ok([]), None::<[_; 0]>);
		assert_cut(
			special(),
			mii(5, 6),
			Err::<[_; 0], _>(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_cut(
			special(),
			mii(6, 7),
			Err::<[_; 0], _>(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_cut(
			special(),
			ie(7, 8),
			Ok([((ee(7, 8)), true)]),
			Some([(mii(4, 6), false), (mii(8, 12), false)]),
		);
		assert_cut(
			special(),
			mii(7, 10),
			Err::<[_; 0], _>(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_cut(
			special(),
			mee(4, 6),
			Ok([(ee(4, 6), false)]),
			Some([
				(mii(4, 4), false),
				(mii(6, 6), false),
				(mee(7, 8), true),
				(mii(8, 12), false),
			]),
		);
	}

	fn assert_cut<const N: usize, const Y: usize, Q, I, K, V>(
		mut before: RangeBoundsMap<I, K, V>,
		to_cut: Q,
		result: Result<[((Bound<I>, Bound<I>), V); Y], TryFromBoundsError>,
		after: Option<[(K, V); N]>,
	) where
		I: Ord + Debug + Copy,
		K: NiceRange<I> + TryFrom<DiscreteBounds<I>> + PartialEq + Debug,
		Q: NiceRange<I>,
		V: PartialEq + Debug + Clone,
	{
		let clone = before.clone();
		match before.cut(to_cut) {
			Ok(iter) => {
				assert_eq!(iter.collect::<Vec<_>>(), result.unwrap());
			}
			Err(x) => {
				assert_eq!(x, result.unwrap_err());
			}
		}
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn gaps_tests() {
		assert_gaps(basic(), ii(50, 60), [ii(50, 60)]);
		assert_gaps(basic(), iu(50), [iu(50)]);
		assert_gaps(basic(), ee(3, 16), [ei(4, 5), ee(7, 14)]);
		assert_gaps(basic(), ei(3, 16), [ei(4, 5), ee(7, 14), ii(16, 16)]);
		assert_gaps(basic(), ue(5), [ee(4, 5)]);
		assert_gaps(basic(), ui(3), []);
		assert_gaps(basic(), ii(5, 5), [ii(5, 5)]);
		assert_gaps(basic(), ii(6, 6), []);
		assert_gaps(basic(), ii(7, 7), []);
		assert_gaps(basic(), ii(8, 8), [ii(8, 8)]);
	}
	fn assert_gaps<const N: usize>(
		map: RangeBoundsMap<i8, AnyRange, bool>,
		outer_range: AnyRange,
		result: [AnyRange; N],
	) {
		assert_eq!(
			map.gaps(outer_range)
				.map(|(start, end)| (start, end))
				.collect::<Vec<_>>(),
			result
		);
	}

	#[test]
	fn insert_merge_touching_tests() {
		assert_insert_merge_touching(
			basic(),
			(ii(0, 4), false),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching(
			basic(),
			(ee(7, 10), false),
			Ok(ie(7, 10)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ie(7, 10), false),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching(
			basic(),
			(ee(7, 11), true),
			Ok(ie(7, 11)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ie(7, 11), true),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching(
			basic(),
			(ee(12, 13), true),
			Ok(ee(12, 13)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(12, 13), true),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching(
			basic(),
			(ee(13, 14), false),
			Ok(ee(13, 16)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(13, 16), false),
			]),
		);
		assert_insert_merge_touching(
			basic(),
			(ee(7, 14), false),
			Ok(ie(7, 16)),
			Some([(ui(4), false), (ee(5, 7), true), (ie(7, 16), false)]),
		);

		assert_insert_merge_touching(
			special(),
			(mee(6, 7), true),
			Err(OverlapOrTryFromBoundsError::TryFromBounds(
				TryFromBoundsError,
			)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching(
			special(),
			(mii(6, 7), true),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching(
			special(),
			(mee(12, 15), true),
			Err(OverlapOrTryFromBoundsError::TryFromBounds(
				TryFromBoundsError,
			)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching(
			special(),
			(mii(12, 15), true),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
	}
	fn assert_insert_merge_touching<const N: usize, I, K, V>(
		mut before: RangeBoundsMap<I, K, V>,
		to_insert: (K, V),
		result: Result<K, OverlapOrTryFromBoundsError>,
		after: Option<[(K, V); N]>,
	) where
		I: Ord + Debug + Copy,
		K: NiceRange<I> + TryFrom<DiscreteBounds<I>> + PartialEq + Debug,
		V: PartialEq + Debug + Clone,
	{
		let clone = before.clone();
		assert_eq!(
			before.insert_merge_touching(to_insert.0, to_insert.1),
			result
		);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}
	#[test]
	fn insert_merge_touching_if_values_equal_tests() {
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ii(0, 4), false),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ee(7, 10), false),
			Ok(ie(7, 10)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ie(7, 10), false),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ee(7, 11), true),
			Ok(ee(7, 11)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(7, 11), true),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ee(12, 13), true),
			Ok(ee(12, 13)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(12, 13), true),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ee(13, 14), true),
			Ok(ee(13, 16)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ee(13, 16), true),
			]),
		);
		assert_insert_merge_touching_if_values_equal(
			basic(),
			(ee(7, 14), false),
			Ok(ie(7, 14)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ie(7, 14), false),
				(ie(14, 16), true),
			]),
		);

		assert_insert_merge_touching_if_values_equal(
			special(),
			(mee(6, 7), true),
			Ok(mee(6, 7)),
			Some([
				(mii(4, 6), false),
				(mee(6, 7), true),
				(mee(7, 8), true),
				(mii(8, 12), false),
			]),
		);
		assert_insert_merge_touching_if_values_equal(
			special(),
			(mii(6, 7), true),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_if_values_equal(
			special(),
			(mee(12, 15), false),
			Err(OverlapOrTryFromBoundsError::TryFromBounds(
				TryFromBoundsError,
			)),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_if_values_equal(
			special(),
			(mii(12, 15), true),
			Err(OverlapOrTryFromBoundsError::Overlap(OverlapError)),
			None::<[_; 0]>,
		);
	}
	fn assert_insert_merge_touching_if_values_equal<const N: usize, I, K, V>(
		mut before: RangeBoundsMap<I, K, V>,
		to_insert: (K, V),
		result: Result<K, OverlapOrTryFromBoundsError>,
		after: Option<[(K, V); N]>,
	) where
		I: Ord + Debug + Copy,
		K: NiceRange<I> + TryFrom<DiscreteBounds<I>> + PartialEq + Debug,
		V: Eq + Debug + Clone,
	{
		let clone = before.clone();
		assert_eq!(
			before.insert_merge_touching_if_values_equal(
				to_insert.0,
				to_insert.1
			),
			result
		);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn insert_merge_overlapping_tests() {
		assert_insert_merge_overlapping(
			basic(),
			(ii(0, 2), true),
			Ok(ui(4)),
			Some([
				(ui(4), true),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_overlapping(
			basic(),
			(ie(14, 16), false),
			Ok(ie(14, 16)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), false),
			]),
		);
		assert_insert_merge_overlapping(
			basic(),
			(ii(6, 11), false),
			Ok(ei(5, 11)),
			Some([(ui(4), false), (ei(5, 11), false), (ie(14, 16), true)]),
		);
		assert_insert_merge_overlapping(
			basic(),
			(ii(15, 18), true),
			Ok(ii(14, 18)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ii(14, 18), true),
			]),
		);
		assert_insert_merge_overlapping(
			basic(),
			(uu(), false),
			Ok(uu()),
			Some([(uu(), false)]),
		);

		assert_insert_merge_overlapping(
			special(),
			(mii(10, 18), true),
			Ok(mii(8, 18)),
			Some([(mii(4, 6), false), (mee(7, 8), true), (mii(8, 18), true)]),
		);
		assert_insert_merge_overlapping(
			special(),
			(mee(10, 18), true),
			Err(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_insert_merge_overlapping(
			special(),
			(mee(8, 12), true),
			Ok(mii(8, 12)),
			Some([(mii(4, 6), false), (mee(7, 8), true), (mii(8, 12), true)]),
		);
		assert_insert_merge_overlapping(
			special(),
			(mee(7, 8), false),
			Ok(mee(7, 8)),
			Some([(mii(4, 6), false), (mee(7, 8), false), (mii(8, 12), false)]),
		);
		assert_insert_merge_overlapping(
			special(),
			(mii(7, 8), false),
			Ok(mii(7, 12)),
			Some([(mii(4, 6), false), (mii(7, 12), false)]),
		);
	}
	fn assert_insert_merge_overlapping<const N: usize, I, K, V>(
		mut before: RangeBoundsMap<I, K, V>,
		to_insert: (K, V),
		result: Result<K, TryFromBoundsError>,
		after: Option<[(K, V); N]>,
	) where
		I: Ord + Debug + Copy,
		K: NiceRange<I> + TryFrom<DiscreteBounds<I>> + PartialEq + Debug,
		V: PartialEq + Debug + Clone,
	{
		let clone = before.clone();
		assert_eq!(
			before.insert_merge_overlapping(to_insert.0, to_insert.1),
			result
		);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn insert_merge_touching_or_overlapping_tests() {
		assert_insert_merge_touching_or_overlapping(
			RangeBoundsMap::from_slice_strict([(ie(1, 4), false)]).unwrap(),
			(ie(0, 1), true),
			Ok(ie(0, 4)),
			Some([(ie(0, 4), true)]),
		);

		//copied from insert_merge_overlapping_tests
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(ii(0, 2), true),
			Ok(ui(4)),
			Some([
				(ui(4), true),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), true),
			]),
		);
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(ie(14, 16), false),
			Ok(ie(14, 16)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ie(14, 16), false),
			]),
		);
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(ii(6, 11), false),
			Ok(ei(5, 11)),
			Some([(ui(4), false), (ei(5, 11), false), (ie(14, 16), true)]),
		);
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(ii(15, 18), true),
			Ok(ii(14, 18)),
			Some([
				(ui(4), false),
				(ee(5, 7), true),
				(ii(7, 7), false),
				(ii(14, 18), true),
			]),
		);
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(uu(), false),
			Ok(uu()),
			Some([(uu(), false)]),
		);
		//the only difference from the insert_merge_overlapping
		assert_insert_merge_touching_or_overlapping(
			basic(),
			(ii(7, 14), false),
			Ok(ee(5, 16)),
			Some([(ui(4), false), (ee(5, 16), false)]),
		);

		//copied from insert_merge_overlapping_tests
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mii(10, 18), true),
			Ok(mii(8, 18)),
			Some([(mii(4, 6), false), (mee(7, 8), true), (mii(8, 18), true)]),
		);
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mee(10, 18), true),
			Err(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mee(8, 12), true),
			Ok(mii(8, 12)),
			Some([(mii(4, 6), false), (mee(7, 8), true), (mii(8, 12), true)]),
		);
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mee(7, 8), false),
			Err(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mii(7, 8), false),
			Ok(mii(7, 12)),
			Some([(mii(4, 6), false), (mii(7, 12), false)]),
		);
		//copied from insert_merge_touching_tests
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mee(6, 7), true),
			Err(TryFromBoundsError),
			None::<[_; 0]>,
		);
		assert_insert_merge_touching_or_overlapping(
			special(),
			(mee(12, 15), true),
			Err(TryFromBoundsError),
			None::<[_; 0]>,
		);
	}
	fn assert_insert_merge_touching_or_overlapping<const N: usize, I, K, V>(
		mut before: RangeBoundsMap<I, K, V>,
		to_insert: (K, V),
		result: Result<K, TryFromBoundsError>,
		after: Option<[(K, V); N]>,
	) where
		I: Ord + Debug + Copy,
		K: NiceRange<I> + TryFrom<DiscreteBounds<I>> + PartialEq + Debug,
		V: PartialEq + Debug + Clone,
	{
		let clone = before.clone();
		assert_eq!(
			before
				.insert_merge_touching_or_overlapping(to_insert.0, to_insert.1),
			result
		);
		match after {
			Some(after) => {
				assert_eq!(
					before,
					RangeBoundsMap::from_slice_strict(after).unwrap()
				)
			}
			None => assert_eq!(before, clone),
		}
	}

	#[test]
	fn config_tests() {
		assert_eq!(config(ie(1, 4), ie(6, 8)), Config::LeftFirstNonOverlapping);
		assert_eq!(config(ie(1, 4), ie(2, 8)), Config::LeftFirstPartialOverlap);
		assert_eq!(config(ie(1, 4), ie(2, 3)), Config::LeftContainsRight);

		assert_eq!(
			config(ie(6, 8), ie(1, 4)),
			Config::RightFirstNonOverlapping
		);
		assert_eq!(
			config(ie(2, 8), ie(1, 4)),
			Config::RightFirstPartialOverlap
		);
		assert_eq!(config(ie(2, 3), ie(1, 4)), Config::RightContainsLeft);
	}

	#[test]
	fn overlaps_tests() {
		for range1 in all_valid_test_bounds() {
			for range2 in all_valid_test_bounds() {
				let our_answer = overlaps(range1, range2);

				let mathematical_definition_of_overlap = NUMBERS_DOMAIN
					.iter()
					.any(|x| range1.contains(x) && range2.contains(x));

				if our_answer != mathematical_definition_of_overlap {
					dbg!(range1, range2);
					dbg!(mathematical_definition_of_overlap, our_answer);
					panic!("Discrepency in overlaps() detected!");
				}
			}
		}
	}

	#[test]
	fn cut_range_tests() {
		for base in all_valid_test_bounds() {
			for cut in all_valid_test_bounds() {
				let cut_result @ CutResult {
					before_cut: b,
					inside_cut: i,
					after_cut: a,
				} = cut_range(base, cut);

				let mut on_left = true;

				// The definition of a cut is: A && NOT B
				for x in NUMBERS_DOMAIN {
					let base_contains = base.contains(x);
					let cut_contains = cut.contains(x);

					if cut_contains {
						on_left = false;
					}

					let invariant = match (base_contains, cut_contains) {
						(false, _) => !con(b, x) && !con(i, x) && !con(a, x),
						(true, false) => {
							if on_left {
								con(b, x) && !con(i, x) && !con(a, x)
							} else {
								!con(b, x) && !con(i, x) && con(a, x)
							}
						}
						(true, true) => !con(b, x) && con(i, x) && !con(a, x),
					};

					if !invariant {
						dbg!(base_contains);
						dbg!(cut_contains);

						dbg!(on_left);

						dbg!(base);
						dbg!(cut);
						dbg!(cut_result);

						dbg!(x);

						panic!("Invariant Broken!");
					}
				}
			}
		}
	}
	fn con(x: Option<(Bound<i8>, Bound<i8>)>, point: &i8) -> bool {
		match x {
			Some(y) => y.contains(point),
			None => false,
		}
	}
	#[test]
	fn cut_range_bounds_should_return_valid_ranges() {
		let result: CutResult<i8> = cut_range(ie(3, 8), ie(5, 8));
		if let Some(x) = result.before_cut {
			assert!(is_valid_range(x));
		}
		if let Some(x) = result.inside_cut {
			assert!(is_valid_range(x));
		}
		if let Some(x) = result.after_cut {
			assert!(is_valid_range(x));
		}

		let result = cut_range(ie(3, 8), ie(3, 5));
		if let Some(x) = result.before_cut {
			assert!(is_valid_range(x));
		}
		if let Some(x) = result.inside_cut {
			assert!(is_valid_range(x));
		}
		if let Some(x) = result.after_cut {
			assert!(is_valid_range(x));
		}
	}

	// Test Helper Functions
	//======================
	fn all_non_overlapping_test_bound_entries() -> Vec<(AnyRange, AnyRange)> {
		let mut output = Vec::new();
		for test_bounds1 in all_valid_test_bounds() {
			for test_bounds2 in all_valid_test_bounds() {
				if !overlaps(test_bounds1, test_bounds2) {
					output.push((test_bounds1, test_bounds2));
				}
			}
		}

		return output;
	}

	fn all_valid_test_bounds() -> Vec<AnyRange> {
		let mut output = Vec::new();

		//bounded-bounded
		output.append(&mut all_finite_bounded_entries());
		//bounded-unbounded
		for start_bound in all_finite_bounded() {
			output.push((start_bound, u()));
		}
		//unbounded-bounded
		for end_bound in all_finite_bounded() {
			output.push((u(), end_bound));
		}
		//unbounded-unbounded
		output.push(uu());

		return output;
	}

	fn all_finite_bounded_entries() -> Vec<(Bound<i8>, Bound<i8>)> {
		let mut output = Vec::new();
		for i in NUMBERS {
			for j in NUMBERS {
				for i_ex in [false, true] {
					for j_ex in [false, true] {
						if j > i || (j == i && !i_ex && !j_ex) {
							output.push((
								finite_bound(*i, i_ex),
								finite_bound(*j, j_ex),
							));
						}
					}
				}
			}
		}
		return output;
	}

	fn all_finite_bounded() -> Vec<Bound<i8>> {
		let mut output = Vec::new();
		for i in NUMBERS {
			for j in 0..=1 {
				output.push(finite_bound(*i, j == 1));
			}
		}
		return output;
	}

	fn finite_bound(x: i8, included: bool) -> Bound<i8> {
		match included {
			false => Bound::Included(x),
			true => Bound::Excluded(x),
		}
	}
}
