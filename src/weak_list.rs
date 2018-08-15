use std::cell::RefCell;
use std::ops::Deref;
use std::rc::{Rc, Weak};

pub struct Handle<T> {
    cur: Rc<Node<T>>,
}

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
    pub fn remove(this: &Self) {
        this.cur.link.borrow_mut().unlink();
    }

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
            let next_link = self.next.take();
            prev.borrow_mut().next = next_link.clone();
            if let Some(next_w) = next_link {
                let next_node = next_w.upgrade().unwrap();
                next_node.link.borrow_mut().prev = Some(prev_w);
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

    /// Push a value and return its `Rc`-like handler.
    pub fn push(&self, value: T) -> Handle<T> {
        use std::mem::replace;

        let node = Rc::new(Node {
            value,
            link: Rc::new(RefCell::new(Link {
                next: self.head.borrow_mut().next.take(),
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
    /// referenced by some `Handle`.
    pub fn clear(&self) {
        self.take_all();
    }

    /// Take a snapshot for all weak-referenced values in the `WeakList`
    /// and upgrade them.
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
        v.iter().for_each(|h| { Handle::remove(&h); });
        v
    }
}

impl<T> Default for WeakList<T> {
    fn default() -> Self {
        Self::new()
    }
}
