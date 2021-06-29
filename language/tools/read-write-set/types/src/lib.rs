// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

mod access;

pub use access::Access;

use move_core_types::{
    account_address::AccountAddress,
    language_storage::StructType,
};
use std::collections::{hash_map::Entry, HashMap};

/// Offset of an access path: either a field, vector index, or global key
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Offset {
    /// Index into contents of a struct by field offset
    Field(usize),
    /// Unknown index into a vector
    VectorIndex,
    /// A type index into global storage. Only follows a field or vector index of type address
    Global(StructType),
}

#[derive(Debug, Clone)]
pub struct TrieNode {
    /// Optional data associated with the parent in the trie
    data: Option<Access>,
    /// Child pointers labeled by offsets
    children: HashMap<Offset, TrieNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    Const(AccountAddress),
    Formal(usize),
}

#[derive(Debug, Clone)]
pub struct AccessPath {
    pub root: Root,
    pub offsets: Vec<Offset>,
}

pub struct ReadWriteSet(HashMap<Root, TrieNode>);

impl TrieNode {
    pub fn new() -> Self {
        Self {
            data: None,
            children: HashMap::new(),
        }
    }
    pub fn entry(&mut self, o: Offset) -> Entry<Offset, TrieNode> {
        self.children.entry(o)
    }

    fn iter_paths_opt<F>(&self, access_path: &AccessPath, mut f: F) -> F
    where
        F: FnMut(&AccessPath, &Option<&Access>) -> Option<()>,
    {
        if f(access_path, &self.data.as_ref()).is_none() {
            return f;
        };
        for (k, v) in self.children.iter() {
            let mut new_ap = access_path.clone();
            new_ap.offsets.push(k.clone());
            f = v.iter_paths_opt(&new_ap, f)
        }
        f
    }
}

impl ReadWriteSet {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn add_access_path(&mut self, access_path: AccessPath, access: Access) {
        let mut node = self.0.entry(access_path.root).or_insert_with(TrieNode::new);
        for offset in access_path.offsets {
            node = node.entry(offset).or_insert_with(TrieNode::new);
        }
        node.data = Some(access);
    }

    pub fn iter_paths_opt<F>(&self, mut f: F)
    where
        F: FnMut(&AccessPath, &Option<&Access>) -> Option<()>,
    {
        for (key, node) in self.0.iter() {
            let access_path = AccessPath {
                root: key.clone(),
                offsets: vec![],
            };
            f = node.iter_paths_opt(&access_path, f);
        }
    }

    pub fn iter_paths<F>(&self, mut f: F)
    where
        F: FnMut(&AccessPath, &Access) -> Option<()>,
    {
        self.iter_paths_opt(|path, access_opt| {
            match access_opt {
                Some(access) => f(path, access),
                None => Some(()),
            }
        })
    }
}
