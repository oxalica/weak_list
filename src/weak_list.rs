use std::ops::Deref;
use std::cell::{Cell, UnsafeCell};
use std::ptr::NonNull;

/// The `Rc`-like handle owning a value,
/// which may have at most one weak reference in a list.
pub struct Handle<T> {
    cur: NonNull<Node<T>>,
}

/// The list of weak references of T.
///
/// Unlike `Vec<Weak<T>>`, once a weak reference in `WeakList` died,
/// immediately, it will be removed from the list and both the space of value
/// and its weak reference will be freed completely.
pub struct WeakList<T> {
    head: Box<UnsafeCell<NodePtr<T>>>,
}

type NodePtr<T> = Option<NonNull<Node<T>>>;

struct Node<T> {
    value: T,
    strong_count: Cell<usize>,
    prev_next: Cell<Option<NonNull<NodePtr<T>>>>,
    next: UnsafeCell<NodePtr<T>>,
}

impl<T> Node<T> {
    unsafe fn new_before(next_ptr: NodePtr<T>, value: T) -> NonNull<Node<T>> {
        let b = Box::new(Node {
            value,
            strong_count: Cell::new(0), // Begin at 0
            prev_next: Cell::new(None),
            next: UnsafeCell::new(next_ptr),
        });
        if let Some(next) = next_ptr {
            let rev_ptr = NonNull::new_unchecked(b.next.get());
            next.as_ref().prev_next.set(Some(rev_ptr));
        }
        NonNull::new_unchecked(Box::into_raw(b))
    }

    unsafe fn unlink(&self) {
        if let Some(mut prev_next) = self.prev_next.take() { // Linked
            *prev_next.as_mut() = *self.next.get();
            if let Some(next) = *self.next.get() { // Has next
                next.as_ref().prev_next.set(Some(prev_next));
            }
        }
    }
}

impl<T> Handle<T> {
    unsafe fn from_raw_node(node: NonNull<Node<T>>) -> Self {
        let count = &node.as_ref().strong_count;
        count.set(count.get() + 1);
        Handle { cur: node }
    }

    /// Detach the value from the list.
    /// It removes and frees the weak reference of it in the list immediately
    /// (if exists).
    pub fn detach(this: &Self) {
        unsafe { this.cur.as_ref().unlink(); }
    }

    /// Try unwrap the value if `this` is the only `Handle` to it.
    ///
    /// If it success, the weak reference of it in the list (if exists) will
    /// also be removed and freed.
    /// Otherwise, `this` will be returned back with nothing happened.
    pub fn try_unwrap(this: Self) -> Result<T, Self> {
        unsafe {
            Self::detach(&this);
            match this.cur.as_ref().strong_count.get() {
                1 => {
                    let b = Box::from_raw(this.cur.as_ptr());
                    ::std::mem::forget(this);
                    Ok(b.value)
                }
                _ => Err(this),
            }
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        unsafe {
            let count = &self.cur.as_ref().strong_count;
            count.set(count.get() + 1);
            Handle { cur: self.cur }
        }
    }
}

impl<T> Deref for Handle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &self.cur.as_ref().value }
    }
}

impl<T> Drop for Handle<T> {
    fn drop(&mut self) {
        unsafe {
            let count = &self.cur.as_ref().strong_count;
            match count.get() {
                1 => {
                    Handle::detach(&self);
                    drop(Box::from_raw(self.cur.as_ptr()));
                },
                x => count.set(x - 1),
            }
        }
    }
}

impl<T> WeakList<T> {
    /// Create an empty list.
    pub fn new() -> Self {
        WeakList {
            head: Box::new(UnsafeCell::new(None)),
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
        unsafe {
            let old_first = *self.head.get();
            let new_first = Node::new_before(old_first, value);
            let head_place = NonNull::new_unchecked(self.head.get());
            new_first.as_ref().prev_next.set(Some(head_place));
            *self.head.get() = Some(new_first);
            Handle::from_raw_node(new_first)
        }
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
        unsafe {
            let mut v = vec![];
            let mut cur = *self.head.get();
            while let Some(cur_node) = cur {
                v.push(Handle::from_raw_node(cur_node));
                cur = *cur_node.as_ref().next.get();
            }
            v
        }
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
    use std::rc::Rc;
    use std::cell::RefCell;

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
