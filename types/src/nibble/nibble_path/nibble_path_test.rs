// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::{arb_internal_nibble_path, skip_common_prefix, NibblePath};
use crate::nibble::Nibble;
use proptest::prelude::*;

#[test]
fn test_nibble_path_fmt() {
    let nibble_path = NibblePath::new(vec![0x12, 0x34, 0x56]);
    assert_eq!(format!("{:?}", nibble_path), "123456");

    let nibble_path = NibblePath::new(vec![0x12, 0x34, 0x50]);
    assert_eq!(format!("{:?}", nibble_path), "123450");

    let nibble_path = NibblePath::new_odd(vec![0x12, 0x34, 0x50]);
    assert_eq!(format!("{:?}", nibble_path), "12345");
}

#[test]
fn test_create_nibble_path_success() {
    let nibble_path = NibblePath::new(vec![0x12, 0x34, 0x56]);
    assert_eq!(nibble_path.num_nibbles(), 6);

    let nibble_path = NibblePath::new(vec![0x12, 0x34, 0x50]);
    assert_eq!(nibble_path.num_nibbles(), 6);

    let nibble_path = NibblePath::new_odd(vec![0x12, 0x34, 0x50]);
    assert_eq!(nibble_path.num_nibbles(), 5);

    let nibble_path = NibblePath::new(vec![]);
    assert_eq!(nibble_path.num_nibbles(), 0);
}

#[test]
#[should_panic(expected = "Last nibble must be 0.")]
fn test_create_nibble_path_failure() {
    let bytes = vec![0x12, 0x34, 0x56];
    let _nibble_path = NibblePath::new_odd(bytes);
}

#[test]
#[should_panic(expected = "Should have odd number of nibbles.")]
fn test_empty_nibble_path() {
    NibblePath::new_odd(vec![]);
}

#[test]
fn test_get_nibble() {
    let bytes = vec![0x12, 0x34];
    let nibble_path = NibblePath::new(bytes);
    assert_eq!(nibble_path.get_nibble(0), Nibble::from(0x01));
    assert_eq!(nibble_path.get_nibble(1), Nibble::from(0x02));
    assert_eq!(nibble_path.get_nibble(2), Nibble::from(0x03));
    assert_eq!(nibble_path.get_nibble(3), Nibble::from(0x04));
}

#[test]
fn test_nibble_iterator() {
    let bytes = vec![0x12, 0x30];
    let nibble_path = NibblePath::new_odd(bytes);
    let mut iter = nibble_path.nibbles();
    assert_eq!(iter.next().unwrap(), Nibble::from(0x01));
    assert_eq!(iter.next().unwrap(), Nibble::from(0x02));
    assert_eq!(iter.next().unwrap(), Nibble::from(0x03));
    assert_eq!(iter.next(), None);
}

#[test]
fn test_get_bit() {
    let bytes = vec![0x01, 0x02];
    let nibble_path = NibblePath::new(bytes);
    assert_eq!(nibble_path.get_bit(0), false);
    assert_eq!(nibble_path.get_bit(1), false);
    assert_eq!(nibble_path.get_bit(2), false);
    assert_eq!(nibble_path.get_bit(7), true);
    assert_eq!(nibble_path.get_bit(8), false);
    assert_eq!(nibble_path.get_bit(14), true);
}

#[test]
fn test_bit_iter() {
    let bytes = vec![0xc3, 0xa0];
    let nibble_path = NibblePath::new_odd(bytes);
    let mut iter = nibble_path.bits();
    // c: 0b1100
    assert_eq!(iter.next(), Some(true));
    assert_eq!(iter.next(), Some(true));
    assert_eq!(iter.next(), Some(false));
    assert_eq!(iter.next(), Some(false));
    // 3: 0b0011
    assert_eq!(iter.next(), Some(false));
    assert_eq!(iter.next(), Some(false));
    assert_eq!(iter.next(), Some(true));
    assert_eq!(iter.next(), Some(true));
    // a: 0b1010
    assert_eq!(iter.next_back(), Some(false));
    assert_eq!(iter.next_back(), Some(true));
    assert_eq!(iter.next_back(), Some(false));
    assert_eq!(iter.next_back(), Some(true));

    assert_eq!(iter.next(), None);
}

#[test]
fn test_visited_nibble_iter() {
    let bytes = vec![0x12, 0x34, 0x56];
    let nibble_path = NibblePath::new(bytes);
    let mut iter = nibble_path.nibbles();
    assert_eq!(iter.next().unwrap(), 0x01.into());
    assert_eq!(iter.next().unwrap(), 0x02.into());
    assert_eq!(iter.next().unwrap(), 0x03.into());
    let mut visited_nibble_iter = iter.visited_nibbles();
    assert_eq!(visited_nibble_iter.next().unwrap(), 0x01.into());
    assert_eq!(visited_nibble_iter.next().unwrap(), 0x02.into());
    assert_eq!(visited_nibble_iter.next().unwrap(), 0x03.into());
}

#[test]
fn test_skip_common_prefix() {
    {
        let nibble_path1 = NibblePath::new(vec![0x12, 0x34, 0x56]);
        let nibble_path2 = NibblePath::new(vec![0x12, 0x34, 0x56]);
        let mut iter1 = nibble_path1.nibbles();
        let mut iter2 = nibble_path2.nibbles();
        assert_eq!(skip_common_prefix(&mut iter1, &mut iter2), 6);
        assert!(iter1.is_finished());
        assert!(iter2.is_finished());
    }
    {
        let nibble_path1 = NibblePath::new(vec![0x12, 0x35]);
        let nibble_path2 = NibblePath::new(vec![0x12, 0x34, 0x56]);
        let mut iter1 = nibble_path1.nibbles();
        let mut iter2 = nibble_path2.nibbles();
        assert_eq!(skip_common_prefix(&mut iter1, &mut iter2), 3);
        assert_eq!(
            iter1.visited_nibbles().get_nibble_path(),
            iter2.visited_nibbles().get_nibble_path()
        );
        assert_eq!(
            iter1.remaining_nibbles().get_nibble_path(),
            NibblePath::new_odd(vec![0x50])
        );
        assert_eq!(
            iter2.remaining_nibbles().get_nibble_path(),
            NibblePath::new_odd(vec![0x45, 0x60])
        );
    }
    {
        let nibble_path1 = NibblePath::new(vec![0x12, 0x34, 0x56]);
        let nibble_path2 = NibblePath::new_odd(vec![0x12, 0x30]);
        let mut iter1 = nibble_path1.nibbles();
        let mut iter2 = nibble_path2.nibbles();
        assert_eq!(skip_common_prefix(&mut iter1, &mut iter2), 3);
        assert_eq!(
            iter1.visited_nibbles().get_nibble_path(),
            iter2.visited_nibbles().get_nibble_path()
        );
        assert_eq!(
            iter1.remaining_nibbles().get_nibble_path(),
            NibblePath::new_odd(vec![0x45, 0x60])
        );
        assert!(iter2.is_finished());
    }
}

prop_compose! {
    fn arb_nibble_path_and_current()(nibble_path in any::<NibblePath>())
        (current in 0..=nibble_path.num_nibbles(),
         nibble_path in Just(nibble_path)) -> (usize, NibblePath) {
        (current, nibble_path)
    }
}

proptest! {
    #[test]
    fn test_push(
        nibble_path in arb_internal_nibble_path(),
        nibble in any::<Nibble>()
    ) {
        let mut new_nibble_path = nibble_path.clone();
        new_nibble_path.push(nibble);
        let mut nibbles: Vec<Nibble> = nibble_path.nibbles().collect();
        nibbles.push(nibble);
        let nibble_path2 = nibbles.into_iter().collect();
        prop_assert_eq!(new_nibble_path, nibble_path2);
    }

    #[test]
    fn test_pop(mut nibble_path in any::<NibblePath>()) {
        let mut nibbles: Vec<Nibble> = nibble_path.nibbles().collect();
        let nibble_from_nibbles = nibbles.pop();
        let nibble_from_nibble_path = nibble_path.pop();
        let nibble_path2 = nibbles.into_iter().collect();
        prop_assert_eq!(nibble_path, nibble_path2);
        prop_assert_eq!(nibble_from_nibbles, nibble_from_nibble_path);
    }

    #[test]
    fn test_last(mut nibble_path in any::<NibblePath>()) {
        let nibble1 = nibble_path.last();
        let nibble2 = nibble_path.pop();
        prop_assert_eq!(nibble1, nibble2);
    }

    #[test]
    fn test_nibble_iter_roundtrip(nibble_path in any::<NibblePath>()) {
        let nibbles = nibble_path.nibbles();
        let nibble_path2 = nibbles.collect();
        prop_assert_eq!(nibble_path, nibble_path2);
    }

    #[test]
    fn test_visited_and_remaining_nibbles((current, nibble_path) in arb_nibble_path_and_current()) {
        let mut nibble_iter = nibble_path.nibbles();
        let mut visited_nibbles = vec![];
        for _ in 0..current {
            visited_nibbles.push(nibble_iter.next().unwrap());
        }
        let visited_nibble_path = nibble_iter.visited_nibbles().get_nibble_path();
        let remaining_nibble_path = nibble_iter.remaining_nibbles().get_nibble_path();
        let visited_iter = visited_nibble_path.nibbles();
        let remaining_iter = remaining_nibble_path.nibbles();
        prop_assert_eq!(visited_nibbles, visited_iter.collect::<Vec<Nibble>>());
        prop_assert_eq!(nibble_iter.collect::<Vec<Nibble>>(), remaining_iter.collect::<Vec<_>>());
   }

   #[test]
    fn test_nibble_iter_to_bit_iter((current, nibble_path) in arb_nibble_path_and_current()) {
        let mut nibble_iter = nibble_path.nibbles();
        (0..current)
            .for_each(|_| {
                nibble_iter.next().unwrap();
            }
        );
        let remaining_nibble_path = nibble_iter.remaining_nibbles().get_nibble_path();
        let remaining_bit_iter = remaining_nibble_path.bits();
        let bit_iter = nibble_iter.bits();
        prop_assert_eq!(remaining_bit_iter.collect::<Vec<bool>>(), bit_iter.collect::<Vec<_>>());
    }
}
