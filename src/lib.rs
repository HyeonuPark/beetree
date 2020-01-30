
use std::rc::{Rc, Weak};
use std::cell::UnsafeCell;
use std::borrow::Borrow;
use std::cmp::{Ord, Eq};
use std::mem;

use arrayvec::ArrayVec;

const MIN_LEN: usize = 4;
const MAX_LEN: usize = MIN_LEN * 2 - 1;

#[derive(Debug)]
pub struct BTreeMap<K, V> {
    token: Token,
    length: usize,
    root: Option<Node<K, V>>,
}

unsafe impl<K: Send, V: Send> Send for BTreeMap<K, V> {}
unsafe impl<K: Sync, V: Sync> Sync for BTreeMap<K, V> {}

impl<K: Ord + Eq, V> BTreeMap<K, V> {
    pub fn new() -> Self {
        BTreeMap {
            token: Token,
            length: 0,
            root: None,
        }
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn get<Q>(&self, query: &Q) -> Option<&V> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        Some(self.root.as_ref()?.get(query, &self.token)?.1)
    }

    pub fn get_mut<Q>(&mut self, query: &Q) -> Option<&mut V> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        Some(self.root.as_mut()?.get_mut(query, &mut self.token)?.1)
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let root = self.root.get_or_insert_with(|| Node::Leaf(Rc::new(Leaf::new())));

        let (sep, right) = match root.insert((key, value), &mut self.token) {
            Insert::Ok => {
                self.length += 1;
                return None
            }
            Insert::Duplicated(_, value) => {
                return Some(value)
            }
            Insert::Split(leaf, intra) => {
                self.length += 1;
                (leaf, intra)
            }
        };
        let left = self.root.take().unwrap();

        let root = Intra {
            heads: Some((left, sep)).into_iter().collect(),
            tail: right,
        };

        self.root = Some(Node::Intra(Box::new(root)));
        None
    }

    pub fn remove<Q>(&mut self, query: &Q) -> Option<V> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        let value = self.root.as_mut()?.remove(query, &mut self.token)?.1;
        self.length -= 1;
        Some(value)
    }
}

#[derive(Debug)]
struct Token;

#[derive(Debug)]
enum Node<K, V> {
    Intra(Box<Intra<K, V>>),
    Leaf(Rc<Leaf<K, V>>),
}

#[derive(Debug)]
struct Intra<K, V> {
    heads: ArrayVec<[(Node<K, V>, Rc<Leaf<K, V>>); MAX_LEN - 1]>,
    tail: Node<K, V>,
}

#[derive(Debug)]
struct Leaf<K, V>(UnsafeCell<LeafData<K, V>>);

#[derive(Debug)]
struct LeafData<K, V> {
    data: ArrayVec<[(K, V); MAX_LEN]>,
    left: Weak<Leaf<K, V>>,
    right: Weak<Leaf<K, V>>,
}

#[derive(Debug)]
enum Insert<K, V> {
    Ok,
    Duplicated(K, V),
    Split(Rc<Leaf<K, V>>, Node<K, V>),
}

impl<K, V> Leaf<K, V> {
    fn new() -> Self {
        Self::with_data(ArrayVec::new())
    }

    fn with_data(data: ArrayVec<[(K, V); MAX_LEN]>) -> Self {
        Leaf(UnsafeCell::new(LeafData {
            data,
            left: Weak::new(),
            right: Weak::new(),
        }))
    }

    fn get<'a>(&self, _: &'a Token) -> &'a LeafData<K, V> {
        unsafe {
            &*self.0.get()
        }
    }

    fn get_mut<'a>(&self, _: &'a mut Token) -> &'a mut LeafData<K, V> {
        unsafe {
            &mut *self.0.get()
        }
    }
}

impl<K: Ord + Eq, V> Node<K, V> {
    fn len(&self, token: &Token) -> usize {
        match self {
            Node::Intra(intra) => intra.heads.len() + 1,
            Node::Leaf(leaf) => leaf.get(token).data.len(),
        }
    }

    fn get<'a, Q>(&self, query: &Q, token: &'a Token) -> Option<(&'a K, &'a V)> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        match self {
            Node::Leaf(leaf) => {
                leaf.get(token).data.iter().find_map(|(k, v)| {
                    if Borrow::borrow(k) == query {
                        Some((k, v))
                    } else {
                        None
                    }
                })
            }
            Node::Intra(intra) => {
                intra.heads.iter()
                    .find(|(_, leaf)| {
                        leaf.get(token).data[0].0.borrow() >= query
                    })
                    .map(|(node, _)| node)
                    .unwrap_or(&intra.tail)
                    .get(query, token)
            }
        }
    }

    fn get_mut<'a, Q>(&mut self, query: &Q, token: &'a mut Token) -> Option<(&'a K, &'a mut V)> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        match self {
            Node::Leaf(leaf) => {
                Leaf::get_mut(&leaf, token).data.iter_mut().find_map(|(k, v)| {
                    if Borrow::borrow(k) == query {
                        Some((&*k, v))
                    } else {
                        None
                    }
                })
            }
            Node::Intra(intra) => {
                intra.heads.iter_mut()
                    .find(|(_, leaf)| {
                        leaf.get(token).data[0].0.borrow() >= query
                    })
                    .map(|(node, _)| node)
                    .unwrap_or(&mut intra.tail)
                    .get_mut(query, token)
            }
        }
    }

    fn insert(&mut self, mut entry: (K, V), token: &mut Token) -> Insert<K, V> {
        match self {
            Node::Leaf(leaf) => {
                let leaf = leaf.get_mut(token);
                match leaf.data.binary_search_by_key(&&entry.0, |(k, v)| k) {
                    Ok(idx) => {
                        mem::swap(&mut leaf.data[idx], &mut entry);
                        Insert::Duplicated(entry.0, entry.1)
                    }
                    Err(idx) if !leaf.data.is_full() => {
                        leaf.data.insert(idx, entry);
                        Insert::Ok
                    }
                    Err(idx) => {
                        let new_data = if idx < MIN_LEN {
                            let new_data = leaf.data.drain(MIN_LEN..).collect();
                            leaf.data.insert(idx, entry);
                            new_data
                        } else {
                            let mut new_data: ArrayVec<_> = leaf.data.drain((MIN_LEN + 1)..).collect();
                            new_data.insert(idx - MIN_LEN, entry);
                            new_data
                        };
                        let new_leaf = Rc::new(Leaf::with_data(new_data));
                        Insert::Split(new_leaf.clone(), Node::Leaf(new_leaf))
                    }
                }
            }
        }
    }

    fn remove<Q>(&mut self, query: &Q, token: &mut Token) -> Option<(K, V)> where
        K: Borrow<Q>,
        Q: Ord + Eq,
    {
        unimplemented!()
    }
}
