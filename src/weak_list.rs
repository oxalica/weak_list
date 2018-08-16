use std::cell::RefCell;
use std::ops::Deref;
use std::rc::{Rc, Weak};

/// The `Rc`-like handle owning a value,
/// which may have at most one weak reference in a list.
pub struct Handle<T> {
    cur: Rc<Node<T>>,
}

/// The list of weak references of T.
///
/// Unlike `Vec<Weak<T>>`, once a weak reference in `WeakList` died,
/// immediately, it will be removed from the list and both the space of value
/// and its weak reference will be freed completely.
pub struct WeakList<T> {
    head: Rc<RefCell<Link<T>>>,
}

struct Node<T> {
    value: T,
    link: Rc<RefCell<Link<T>>>,
}

struct Link<T> {
    next: Option<Weak<Node<T>>>,
    prev: Option<Weak<RefCell<Link<T>>>>,
}

impl<T> Handle<T> {
    /// Detach the value from the list.
    /// It removes and frees the weak reference of it in the list immediately
    /// (if exists).
    pub fn detach(this: &Self) {
        this.cur.link.borrow_mut().unlink();
    }

    /// Try unwrap the value if `this` is the only `Handle` to it.
    ///
    /// If it success, the weak reference of it in the list (if exists) will
    /// also be removed and freed.
    /// Otherwise, `this` will be returned back with nothing happened.
    pub fn try_unwrap(this: Self) -> Result<T, Self> {
        match Rc::try_unwrap(this.cur) {
            Ok(node) => Ok(node.value),
            Err(cur) => Err(Handle { cur }),
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Handle { cur: Rc::clone(&self.cur) }
    }
}

impl<T> Deref for Handle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.cur.value
    }
}

impl<T> Link<T> {
    fn new() -> Self {
        Link {
            next: None,
            prev: None,
        }
    }

    fn unlink(&mut self) {
        // If in list.
        if let Some(prev_w) = self.prev.take() {
            let prev = prev_w.upgrade().unwrap();
            let mut prev_link = prev.borrow_mut();
            match self.next.take() {
                None => prev_link.next = None,
                Some(next_w) => {
                    let next_node = next_w.upgrade().unwrap();
                    prev_link.next = Some(Rc::downgrade(&next_node));
                    next_node.link.borrow_mut().prev = Some(prev_w);
                }
            }
        }
    }
}

impl<T> Drop for Link<T> {
    fn drop(&mut self) {
        self.unlink();
    }
}

impl<T> WeakList<T> {
    /// Create an empty list.
    pub fn new() -> Self {
        WeakList {
            head: Rc::new(RefCell::new(Link::new()))
        }
    }

    /// Wrap a value into `Handle` and push the weak reference into the list.
    ///
    /// # Warning
    /// When it returns, the `Handle` is currently the only strong reference
    /// to the value. So discard the return value like `list.new_elem(value);`
    /// will cause the value being dropped and removed from `list` immediately,
    /// which is quite meaningless.
    pub fn new_elem(&self, value: T) -> Handle<T> {
        use std::mem::replace;

        let node = Rc::new(Node {
            value,
            link: Rc::new(RefCell::new(Link {
                next: self.head.borrow_mut().next.clone(),
                prev: Some(Rc::downgrade(&self.head)),
            })),
        });
        let mut head = self.head.borrow_mut();
        if let Some(old_w) = replace(&mut head.next, Some(Rc::downgrade(&node))) {
            let old_node = old_w.upgrade().unwrap();
            old_node.link.borrow_mut().prev = Some(Rc::downgrade(&node.link));
        }
        Handle { cur: node }
    }

    /// Clear the list and free spaces for all weak references.
    ///
    /// Note that it never cause the drop of any value.
    /// All values existing in the `WeakList` must still be strongly
    /// referenced by some `Handle`s outside.
    pub fn clear(&self) {
        self.take_all();
    }

    /// Take a snapshot for all weak-referenced values in the `WeakList`
    /// and upgrade them.
    ///
    /// It will not change the list.
    pub fn upgrade_all(&self) -> Vec<Handle<T>> {
        let mut v = vec![];
        let mut cur = self.head.borrow().next.clone();
        while let Some(node_w) = cur {
            let node = node_w.upgrade().unwrap();
            cur = node.link.borrow().next.clone();
            v.push(Handle { cur: node });
        }
        v
    }

    /// The same as `upgrade_all`, except it clears the list before return.
    pub fn take_all(&self) -> Vec<Handle<T>> {
        let v = self.upgrade_all();
        v.iter().for_each(|h| Handle::detach(&h));
        v
    }
}

impl<T> Default for WeakList<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct S {
        value: i32,
        buf: Rc<RefCell<Vec<i32>>>,
    }

    impl Drop for S {
        fn drop(&mut self) {
            self.buf.borrow_mut().push(self.value);
        }
    }

    #[test]
    fn basic_test() {
        use std::mem::replace;

        let buf = Rc::new(RefCell::new(vec![]));
        let get_last_dropped = || replace(&mut *buf.borrow_mut(), vec![]);
        let new_s = |value| S { value, buf: Rc::clone(&buf) };
        let get_values = |v: &[Handle<S>]| -> Vec<i32> {
            v.iter().map(|h| h.value).collect::<Vec<i32>>()
        };

        let h5;
        {
            let ls = WeakList::new();
            let get_snapshot = || get_values(&ls.upgrade_all());
            let mut handles = vec![];

            handles.push(ls.new_elem(new_s(1)));
            handles.push(ls.new_elem(new_s(2)));
            handles.push(ls.new_elem(new_s(3)));
            assert_eq!(get_values(&handles), [1, 2, 3]);
            assert_eq!(get_snapshot(), [3, 2, 1]);
            assert_eq!(get_last_dropped(), []);

            handles.pop();
            assert_eq!(get_values(&handles), [1, 2]);
            assert_eq!(get_snapshot(), [2, 1]);
            assert_eq!(get_last_dropped(), [3]);

            ls.new_elem(new_s(4)); // Immediately drop handle.
            assert_eq!(get_snapshot(), [2, 1]);
            assert_eq!(get_last_dropped(), [4]);

            Handle::detach(&handles[0]); // Remove `1` from list.
            assert_eq!(get_values(&handles), [1, 2]); // Handles will not change.
            assert_eq!(get_snapshot(), [2]);
            assert_eq!(get_last_dropped(), []); // No drop now.

            handles.remove(0); // Drop handle to `1`.
            assert_eq!(get_values(&handles), [2]);
            assert_eq!(get_snapshot(), [2]);
            assert_eq!(get_last_dropped(), [1]);

            ls.clear();
            assert_eq!(get_snapshot(), []);
            assert_eq!(get_last_dropped(), []); // `clear` never cause drop.

            handles.clear();
            assert_eq!(get_last_dropped(), [2]);

            handles.push(ls.new_elem(new_s(5)));
            handles = ls.take_all();
            assert_eq!(get_values(&handles), [5]);
            assert_eq!(get_snapshot(), []); // `take_all` remove all elems from list.
            assert_eq!(get_last_dropped(), []);

            h5 = Handle::clone(&handles[0]);
        }
        assert_eq!(h5.value, 5); // Handle can outlive the list
        assert_eq!(get_last_dropped(), []);

        drop(h5);
        assert_eq!(get_last_dropped(), [5]);
    }
}
