// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module defines traits and implementations of
//! [cryptographic hash functions](https://en.wikipedia.org/wiki/Cryptographic_hash_function)
//! for the Diem project.
//!
//! It is designed to help authors protect against two types of real world attacks:
//!
//! 1. **Semantic Ambiguity**: imagine that Alice has a private key and is using
//!    two different applications, X and Y. X asks Alice to sign a message saying
//!    "I am Alice". Alice accepts to sign this message in the context of X. However,
//!    unbeknownst to Alice, in application Y, messages beginning with the letter "I"
//!    represent transfers. " am " represents a transfer of 500 coins and "Alice"
//!    can be interpreted as a destination address. When Alice signed the message she
//!    needed to be aware of how other applications might interpret that message.
//!
//! 2. **Format Ambiguity**: imagine a program that hashes a pair of strings.
//!    To hash the strings `a` and `b` it hashes `a + "||" + b`. The pair of
//!    strings `a="foo||", b = "bar"` and `a="foo", b = "||bar"` result in the
//!    same input to the hash function and therefore the same hash. This
//!    creates a collision.
//!
//! Regarding (1), this library makes it easy for Diem developers to create as
//! many new "hashable" Rust types as needed so that each Rust type hashed and signed
//! in Diem has a unique meaning, that is, unambiguously captures the intent of a signer.
//!
//! Regarding (2), this library provides the `CryptoHasher` abstraction to easily manage
//! cryptographic seeds for hashing. Hashing seeds aim to ensure that
//! the hashes of values of a given type `MyNewStruct` never collide with hashes of values
//! from another type.
//!
//! Finally, to prevent format ambiguity within a same type `MyNewStruct` and facilitate protocol
//! specifications, we use [Binary Canonical Serialization (BCS)](https://docs.rs/bcs/)
//! as the recommended solution to write Rust values into a hasher.
//!
//! # Quick Start
//!
//! To obtain a `hash()` method for any new type `MyNewStruct`, it is (strongly) recommended to
//! use the derive macros of `serde` and `diem_crypto_derive` as follows:
//! ```
//! use diem_crypto::hash::CryptoHash;
//! use diem_crypto_derive::{CryptoHasher, BCSCryptoHash};
//! use serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, CryptoHasher, BCSCryptoHash)]
//! struct MyNewStruct { /*...*/ }
//!
//! let value = MyNewStruct { /*...*/ };
//! value.hash();
//! ```
//!
//! Under the hood, this will generate a new implementation `MyNewStructHasher` for the trait
//! `CryptoHasher` and implement the trait `CryptoHash` for `MyNewStruct` using BCS.
//!
//! # Implementing New Hashers
//!
//! The trait `CryptoHasher` captures the notion of a pre-seeded hash function, aka a "hasher".
//! New implementations can be defined in two ways.
//!
//! ## Derive macro (recommended)
//!
//! For any new structure `MyNewStruct` that needs to be hashed, it is recommended to simply
//! use the derive macro [`CryptoHasher`](https://doc.rust-lang.org/reference/procedural-macros.html).
//!
//! ```
//! use diem_crypto_derive::CryptoHasher;
//! use serde::Deserialize;
//! #[derive(Deserialize, CryptoHasher)]
//! #[serde(rename = "OptionalCustomSerdeName")]
//! struct MyNewStruct { /*...*/ }
//! ```
//!
//! The macro `CryptoHasher` will define a hasher automatically called `MyNewStructHasher`, and derive a salt
//! using the name of the type as seen by the Serde library. In the example above, this name
//! was changed using the Serde parameter `rename`: the salt will be based on the value `OptionalCustomSerdeName`
//! instead of the default name `MyNewStruct`.
//!
//! ## Customized hashers
//!
//! **IMPORTANT:** Do NOT use this for new code unless you know what you are doing.
//!
//! This library also provides a few customized hashers defined in the code as follows:
//!
//! ```
//! # // To get around that there's no way to doc-test a non-exported macro:
//! # macro_rules! define_hasher { ($e:expr) => () }
//! define_hasher! { (MyNewDataHasher, MY_NEW_DATA_HASHER, MY_NEW_DATA_SEED, b"MyUniqueSaltString") }
//! ```
//!
//! # Using a hasher directly
//!
//! **IMPORTANT:** Do NOT use this for new code unless you know what you are doing.
//!
//! ```
//! use diem_crypto::hash::{CryptoHasher, TestOnlyHasher};
//!
//! let mut hasher = TestOnlyHasher::default();
//! hasher.update("Test message".as_bytes());
//! let hash_value = hasher.finish();
//! ```
#![allow(clippy::integer_arithmetic)]
use bytes::Bytes;
use hex::FromHex;
use mirai_annotations::*;
use once_cell::sync::{Lazy, OnceCell};
#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use rand::{rngs::OsRng, Rng};
use serde::{de, ser};
use std::{
    self,
    convert::{AsRef, TryFrom},
    fmt,
    str::FromStr,
};
use tiny_keccak::{Hasher, Sha3};

/// A prefix used to begin the salt of every diem hashable structure. The salt
/// consists in this global prefix, concatenated with the specified
/// serialization name of the struct.
pub(crate) const DIEM_HASH_PREFIX: &[u8] = b"DIEM::";

/// Output value of our hash function. Intentionally opaque for safety and modularity.
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct HashValue {
    hash: [u8; HashValue::LENGTH],
}

impl HashValue {
    /// The length of the hash in bytes.
    pub const LENGTH: usize = 32;
    /// The length of the hash in bits.
    pub const LENGTH_IN_BITS: usize = Self::LENGTH * 8;

    /// Create a new [`HashValue`] from a byte array.
    pub fn new(hash: [u8; HashValue::LENGTH]) -> Self {
        HashValue { hash }
    }

    /// Create from a slice (e.g. retrieved from storage).
    pub fn from_slice<T: AsRef<[u8]>>(bytes: T) -> Result<Self, HashValueParseError> {
        <[u8; Self::LENGTH]>::try_from(bytes.as_ref())
            .map_err(|_| HashValueParseError)
            .map(Self::new)
    }

    /// Dumps into a vector.
    pub fn to_vec(&self) -> Vec<u8> {
        self.hash.to_vec()
    }

    /// Creates a zero-initialized instance.
    pub const fn zero() -> Self {
        HashValue {
            hash: [0; HashValue::LENGTH],
        }
    }

    /// Create a cryptographically random instance.
    pub fn random() -> Self {
        let mut rng = OsRng;
        let hash: [u8; HashValue::LENGTH] = rng.gen();
        HashValue { hash }
    }

    /// Creates a random instance with given rng. Useful in unit tests.
    pub fn random_with_rng<R: Rng>(rng: &mut R) -> Self {
        let hash: [u8; HashValue::LENGTH] = rng.gen();
        HashValue { hash }
    }

    /// Convenience function that computes a `HashValue` internally equal to
    /// the sha3_256 of a byte buffer. It will handle hasher creation, data
    /// feeding and finalization.
    ///
    /// Note this will not result in the `<T as CryptoHash>::hash()` for any
    /// reasonable struct T, as this computes a sha3 without any ornaments.
    pub fn sha3_256_of(buffer: &[u8]) -> Self {
        let mut sha3 = Sha3::v256();
        sha3.update(buffer);
        HashValue::from_keccak(sha3)
    }

    #[cfg(test)]
    pub fn from_iter_sha3<'a, I>(buffers: I) -> Self
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut sha3 = Sha3::v256();
        for buffer in buffers {
            sha3.update(buffer);
        }
        HashValue::from_keccak(sha3)
    }

    fn as_ref_mut(&mut self) -> &mut [u8] {
        &mut self.hash[..]
    }

    fn from_keccak(state: Sha3) -> Self {
        let mut hash = Self::zero();
        state.finalize(hash.as_ref_mut());
        hash
    }

    /// Returns the `index`-th bit in the bytes.
    pub fn bit(&self, index: usize) -> bool {
        assume!(index < Self::LENGTH_IN_BITS); // assumed precondition
        let pos = index / 8;
        let bit = 7 - index % 8;
        (self.hash[pos] >> bit) & 1 != 0
    }

    /// Returns the `index`-th nibble in the bytes.
    pub fn nibble(&self, index: usize) -> u8 {
        assume!(index < Self::LENGTH * 2); // assumed precondition
        let pos = index / 2;
        let shift = if index % 2 == 0 { 4 } else { 0 };
        (self.hash[pos] >> shift) & 0x0f
    }

    /// Returns a `HashValueBitIterator` over all the bits that represent this `HashValue`.
    pub fn iter_bits(&self) -> HashValueBitIterator<'_> {
        HashValueBitIterator::new(self)
    }

    /// Constructs a `HashValue` from an iterator of bits.
    pub fn from_bit_iter(
        iter: impl ExactSizeIterator<Item = bool>,
    ) -> Result<Self, HashValueParseError> {
        if iter.len() != Self::LENGTH_IN_BITS {
            return Err(HashValueParseError);
        }

        let mut buf = [0; Self::LENGTH];
        for (i, bit) in iter.enumerate() {
            if bit {
                buf[i / 8] |= 1 << (7 - i % 8);
            }
        }
        Ok(Self::new(buf))
    }

    /// Returns the length of common prefix of `self` and `other` in bits.
    pub fn common_prefix_bits_len(&self, other: HashValue) -> usize {
        self.iter_bits()
            .zip(other.iter_bits())
            .take_while(|(x, y)| x == y)
            .count()
    }

    /// Full hex representation of a given hash value.
    pub fn to_hex(&self) -> String {
        format!("{:x}", self)
    }

    /// Parse a given hex string to a hash value.
    pub fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, HashValueParseError> {
        <[u8; Self::LENGTH]>::from_hex(hex)
            .map_err(|_| HashValueParseError)
            .map(Self::new)
    }

    /// Create a hash value whose contents are just the given integer. Useful for
    /// generating basic mock hash values.
    ///
    /// Ex: HashValue::from_u64(0x1234) => HashValue([0, .., 0, 0x12, 0x34])
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn from_u64(v: u64) -> Self {
        let mut hash = [0u8; Self::LENGTH];
        let bytes = v.to_be_bytes();
        hash[Self::LENGTH - bytes.len()..].copy_from_slice(&bytes[..]);
        Self::new(hash)
    }
}

impl ser::Serialize for HashValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_hex())
        } else {
            // In order to preserve the Serde data model and help analysis tools,
            // make sure to wrap our value in a container with the same name
            // as the original type.
            serializer
                .serialize_newtype_struct("HashValue", serde_bytes::Bytes::new(&self.hash[..]))
        }
    }
}

impl<'de> de::Deserialize<'de> for HashValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let encoded_hash = <String>::deserialize(deserializer)?;
            HashValue::from_hex(encoded_hash.as_str())
                .map_err(<D::Error as ::serde::de::Error>::custom)
        } else {
            // See comment in serialize.
            #[derive(::serde::Deserialize)]
            #[serde(rename = "HashValue")]
            struct Value<'a>(&'a [u8]);

            let value = Value::deserialize(deserializer)?;
            Self::from_slice(value.0).map_err(<D::Error as ::serde::de::Error>::custom)
        }
    }
}

impl Default for HashValue {
    fn default() -> Self {
        HashValue::zero()
    }
}

impl AsRef<[u8; HashValue::LENGTH]> for HashValue {
    fn as_ref(&self) -> &[u8; HashValue::LENGTH] {
        &self.hash
    }
}

impl std::ops::Deref for HashValue {
    type Target = [u8; Self::LENGTH];

    fn deref(&self) -> &Self::Target {
        &self.hash
    }
}

impl std::ops::Index<usize> for HashValue {
    type Output = u8;

    fn index(&self, s: usize) -> &u8 {
        self.hash.index(s)
    }
}

impl fmt::Binary for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.hash {
            write!(f, "{:08b}", byte)?;
        }
        Ok(())
    }
}

impl fmt::LowerHex for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.hash {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl fmt::Debug for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HashValue(")?;
        <Self as fmt::LowerHex>::fmt(self, f)?;
        write!(f, ")")?;
        Ok(())
    }
}

/// Will print shortened (4 bytes) hash
impl fmt::Display for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for byte in self.hash.iter().take(4) {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl From<HashValue> for Bytes {
    fn from(value: HashValue) -> Bytes {
        Bytes::copy_from_slice(value.hash.as_ref())
    }
}

impl FromStr for HashValue {
    type Err = HashValueParseError;

    fn from_str(s: &str) -> Result<Self, HashValueParseError> {
        HashValue::from_hex(s)
    }
}

/// Parse error when attempting to construct a HashValue
#[derive(Clone, Copy, Debug)]
pub struct HashValueParseError;

impl fmt::Display for HashValueParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "unable to parse HashValue")
    }
}

impl std::error::Error for HashValueParseError {}

/// An iterator over `HashValue` that generates one bit for each iteration.
pub struct HashValueBitIterator<'a> {
    /// The reference to the bytes that represent the `HashValue`.
    hash_bytes: &'a [u8],
    pos: std::ops::Range<usize>,
    // invariant hash_bytes.len() == HashValue::LENGTH;
    // invariant pos.end == hash_bytes.len() * 8;
}

impl<'a> HashValueBitIterator<'a> {
    /// Constructs a new `HashValueBitIterator` using given `HashValue`.
    fn new(hash_value: &'a HashValue) -> Self {
        HashValueBitIterator {
            hash_bytes: hash_value.as_ref(),
            pos: (0..HashValue::LENGTH_IN_BITS),
        }
    }

    /// Returns the `index`-th bit in the bytes.
    fn get_bit(&self, index: usize) -> bool {
        assume!(index < self.pos.end); // assumed precondition
        assume!(self.hash_bytes.len() == HashValue::LENGTH); // invariant
        assume!(self.pos.end == self.hash_bytes.len() * 8); // invariant
        let pos = index / 8;
        let bit = 7 - index % 8;
        (self.hash_bytes[pos] >> bit) & 1 != 0
    }
}

impl<'a> std::iter::Iterator for HashValueBitIterator<'a> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        self.pos.next().map(|x| self.get_bit(x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.pos.size_hint()
    }
}

impl<'a> std::iter::DoubleEndedIterator for HashValueBitIterator<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.pos.next_back().map(|x| self.get_bit(x))
    }
}

impl<'a> std::iter::ExactSizeIterator for HashValueBitIterator<'a> {}

/// A type that can be cryptographically hashed to produce a `HashValue`.
///
/// In most cases, this trait should not be implemented manually but rather derived using
/// the macros `serde::Serialize`, `CryptoHasher`, and `BCSCryptoHash`.
pub trait CryptoHash {
    /// The associated `Hasher` type which comes with a unique salt for this type.
    type Hasher: CryptoHasher;

    /// Hashes the object and produces a `HashValue`.
    fn hash(&self) -> HashValue;
}

/// A trait for representing the state of a cryptographic hasher.
pub trait CryptoHasher: Default + std::io::Write {
    /// the seed used to initialize hashing `Self` before the serialization bytes of the actual value
    fn seed() -> &'static [u8; 32];

    /// Write bytes into the hasher.
    fn update(&mut self, bytes: &[u8]);

    /// Finish constructing the [`HashValue`].
    fn finish(self) -> HashValue;
}

/// The default hasher underlying generated implementations of `CryptoHasher`.
#[doc(hidden)]
#[derive(Clone)]
pub struct DefaultHasher {
    state: Sha3,
}

impl DefaultHasher {
    #[doc(hidden)]
    /// This function does not return a HashValue in the sense of our usual
    /// hashes, but a construction of initial bytes that are fed into any hash
    /// provided we're passed  a (bcs) serialization name as argument.
    pub fn prefixed_hash(buffer: &[u8]) -> [u8; HashValue::LENGTH] {
        // The salt is initial material we prefix to actual value bytes for
        // domain separation. Its length is variable.
        let salt: Vec<u8> = [DIEM_HASH_PREFIX, buffer].concat();
        // The seed is a fixed-length hash of the salt, thereby preventing
        // suffix attacks on the domain separation bytes.
        HashValue::sha3_256_of(&salt[..]).hash
    }

    #[doc(hidden)]
    pub fn new(typename: &[u8]) -> Self {
        let mut state = Sha3::v256();
        if !typename.is_empty() {
            state.update(&Self::prefixed_hash(typename));
        }
        DefaultHasher { state }
    }

    #[doc(hidden)]
    pub fn update(&mut self, bytes: &[u8]) {
        self.state.update(bytes);
    }

    #[doc(hidden)]
    pub fn finish(self) -> HashValue {
        let mut hasher = HashValue::default();
        self.state.finalize(hasher.as_ref_mut());
        hasher
    }
}

impl fmt::Debug for DefaultHasher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DefaultHasher: state = Sha3")
    }
}

macro_rules! define_hasher {
    (
        $(#[$attr:meta])*
        ($hasher_type: ident, $hasher_name: ident, $seed_name: ident, $salt: expr)
    ) => {

        #[derive(Clone, Debug)]
        $(#[$attr])*
        pub struct $hasher_type(DefaultHasher);

        impl $hasher_type {
            fn new() -> Self {
                $hasher_type(DefaultHasher::new($salt))
            }
        }

        static $hasher_name: Lazy<$hasher_type> = Lazy::new(|| { $hasher_type::new() });
        static $seed_name: OnceCell<[u8; 32]> = OnceCell::new();

        impl Default for $hasher_type {
            fn default() -> Self {
                $hasher_name.clone()
            }
        }

        impl CryptoHasher for $hasher_type {
            fn seed() -> &'static [u8;32] {
                $seed_name.get_or_init(|| {
                    DefaultHasher::prefixed_hash($salt)
                })
            }

            fn update(&mut self, bytes: &[u8]) {
                self.0.update(bytes);
            }

            fn finish(self) -> HashValue {
                self.0.finish()
            }
        }

        impl std::io::Write for $hasher_type {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
    };
}

define_hasher! {
    /// The hasher used to compute the hash of an internal node in the transaction accumulator.
    (
        TransactionAccumulatorHasher,
        TRANSACTION_ACCUMULATOR_HASHER,
        TRANSACTION_ACCUMULATOR_SEED,
        b"TransactionAccumulator"
    )
}

define_hasher! {
    /// The hasher used to compute the hash of an internal node in the event accumulator.
    (
        EventAccumulatorHasher,
        EVENT_ACCUMULATOR_HASHER,
        EVENT_ACCUMULATOR_SEED,
        b"EventAccumulator"
    )
}

define_hasher! {
    /// The hasher used to compute the hash of an internal node in the Sparse Merkle Tree.
    (
        SparseMerkleInternalHasher,
        SPARSE_MERKLE_INTERNAL_HASHER,
        SPARSE_MERKLE_INTERNAL_SEED,
        b"SparseMerkleInternal"
    )
}

define_hasher! {
    /// The hasher used to compute the hash of an internal node in the transaction accumulator.
    (
        VoteProposalHasher,
        VOTE_PROPOSAL_HASHER,
        VOTE_PROPOSAL_SEED,
        b"VoteProposalHasher"
    )
}

define_hasher! {
    /// The hasher used only for testing. It doesn't have a salt.
    (TestOnlyHasher, TEST_ONLY_HASHER, TEST_ONLY_SEED, b"")
}

fn create_literal_hash(word: &str) -> HashValue {
    let mut s = word.as_bytes().to_vec();
    assert!(s.len() <= HashValue::LENGTH);
    s.resize(HashValue::LENGTH, 0);
    HashValue::from_slice(&s).expect("Cannot fail")
}

/// Placeholder hash of `Accumulator`.
pub static ACCUMULATOR_PLACEHOLDER_HASH: Lazy<HashValue> =
    Lazy::new(|| create_literal_hash("ACCUMULATOR_PLACEHOLDER_HASH"));

/// Placeholder hash of `SparseMerkleTree`.
pub static SPARSE_MERKLE_PLACEHOLDER_HASH: Lazy<HashValue> =
    Lazy::new(|| create_literal_hash("SPARSE_MERKLE_PLACEHOLDER_HASH"));

/// Block id reserved as the id of parent block of the genesis block.
pub static PRE_GENESIS_BLOCK_ID: Lazy<HashValue> =
    Lazy::new(|| create_literal_hash("PRE_GENESIS_BLOCK_ID"));

/// Genesis block id is used as a parent of the very first block executed by the executor.
pub static GENESIS_BLOCK_ID: Lazy<HashValue> = Lazy::new(|| {
    // This maintains the invariant that block.id() == block.hash(), for
    // the genesis block and allows us to (de/)serialize it consistently
    HashValue::new([
        0x5e, 0x10, 0xba, 0xd4, 0x5b, 0x35, 0xed, 0x92, 0x9c, 0xd6, 0xd2, 0xc7, 0x09, 0x8b, 0x13,
        0x5d, 0x02, 0xdd, 0x25, 0x9a, 0xe8, 0x8a, 0x8d, 0x09, 0xf4, 0xeb, 0x5f, 0xba, 0xe9, 0xa6,
        0xf6, 0xe4,
    ])
});

/// Provides a test_only_hash() method that can be used in tests on types that implement
/// `serde::Serialize`.
///
/// # Example
/// ```
/// use diem_crypto::hash::TestOnlyHash;
///
/// b"hello world".test_only_hash();
/// ```
pub trait TestOnlyHash {
    /// Generates a hash used only for tests.
    fn test_only_hash(&self) -> HashValue;
}

impl<T: ser::Serialize + ?Sized> TestOnlyHash for T {
    fn test_only_hash(&self) -> HashValue {
        let bytes = bcs::to_bytes(self).expect("serialize failed during hash.");
        let mut hasher = TestOnlyHasher::default();
        hasher.update(&bytes);
        hasher.finish()
    }
}
