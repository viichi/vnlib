
use std::sync::{Mutex, Condvar};
use std::rc::Rc;
use std::cell::Cell;
use std::time::Instant;
use std::ptr;

pub struct Core {
    pub msgs: Mutex<MsgQueue>,
    pub cond: Condvar,
}

impl Core {
    pub fn new() -> Core {
        Core {
            msgs: Mutex::new(MsgQueue::new()),
            cond: Condvar::new(),
        }
    }

    pub fn post<T>(&self, msg: T) where T: FnOnce() + Send + 'static {
        let mut msgs = self.msgs.lock().unwrap();
        msgs.push(msg);
        if msgs.state == State::Waiting {
            msgs.state = State::MsgArrived;
            self.cond.notify_one();
        }
    }

    pub fn stop(&self) {
        let mut msgs = self.msgs.lock().unwrap();
        if msgs.state == State::Waiting {
            self.cond.notify_one();
        }
        msgs.state = State::Stopping;
    }
}

pub struct MsgQueue {
    list: Option<(Box<Action>, *mut Action)>,
    pub state: State,
}

unsafe impl Send for MsgQueue {}

impl MsgQueue {
    fn new() -> MsgQueue {
        MsgQueue {
            list: None,
            state: State::Stopped,
        }
    }

    pub fn drain(&mut self) -> Option<Box<Action>> {
        match self.list.take() {
            Some((head,_)) => Some(head),
            None => None,
        }
    }

    fn push<T>(&mut self, t: T) where T: FnOnce() + Send + 'static {
        let mut node: Box<Action> = Box::new(ActionNode {
            f: Some(t), next: None,
        });
        let tail = node.as_mut() as *mut _;
        match self.list {
            Some((_, ref mut t)) => unsafe {
                 (**t).set_next(node); *t = tail;
            },
            None => self.list = Some((node, tail)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Stopping,
    Running,
    Waiting,
    MsgArrived,
}

pub trait Action : Send {
    fn process(&mut self) -> Option<Box<Action>>;
    fn set_next(&mut self, msg: Box<Action>);
}

struct ActionNode<T> {
    f: Option<T>,
    next: Option<Box<Action>>,
}

impl<T> Action for ActionNode<T> where T: FnOnce() + Send {
    fn process(&mut self) -> Option<Box<Action>> {
        match self.f.take() {
            Some(t) => t(),
            None => unreachable!(),
        }
        self.next.take()
    }

    fn set_next(&mut self, msg: Box<Action>) {
        self.next = Some(msg);
    }
}

pub struct TimedActionNode {
    time: Cell<Instant>,
    index: Cell<usize>,
}

impl TimedActionNode {
    pub fn new() -> TimedActionNode {
        TimedActionNode {
            time: Cell::new(Instant::now()),
            index: Cell::new(0),
        }
    }
}

pub trait TimedAction {
    fn node(&self) -> &TimedActionNode;
    fn process(&self) -> Option<Instant>;
}

pub struct TimedActionBinaryHeap {
    data: Vec<Rc<TimedAction>>
}

impl TimedActionBinaryHeap {
    pub fn new() -> TimedActionBinaryHeap {
        TimedActionBinaryHeap {
            data: Vec::new(),
        }
    }

    pub fn push(&mut self, act: Rc<TimedAction>, time: Instant) {
        let index = self.data.len();
        {
            let node = act.node();
            node.time.set(time);
            node.index.set(index);
        }
        self.data.push(act);
        self.sift_up(index);
    }

    /*
    pub fn pop(&mut self) -> Option<Rc<TimedAction>> {
        match self.data.len() {
            0 => None,
            1 => self.data.pop(),
            n => unsafe {
                self.swap(0, n - 1);
                let t = self.data.pop();
                self.sift_down(0);
                t
            },
        }
    }
    */

    pub fn peek(&self, time: Instant) -> Option<Rc<TimedAction>> {
        if self.data.is_empty() {
            None
        }
        else {
            let ta = unsafe { self.data.get_unchecked(0) };
            if ta.node().time.get() <= time {
                Some(ta.clone())
            }
            else {
                None
            }
        }
    }

    pub fn peek_time(&self) -> Option<Instant> {
        if self.data.is_empty() {
            None
        }
        else {
            unsafe { Some(self.data.get_unchecked(0).node().time.get()) }
        }
    }

    pub fn adjust(&mut self, node: &TimedActionNode, time: Instant) {
        node.time.set(time);
        let index = node.index.get();
        if self.sift_up(index) == index {
            self.sift_down(index);
        }
    }

    pub fn remove(&mut self, node: &TimedActionNode) {
        let index = node.index.get();
        let last = self.data.len() - 1;
        if index == last {
            self.data.pop();
        }
        else {
            unsafe { self.swap(index, last); }
            self.data.pop();
            self.sift_down(index);
        }
    }

    fn sift_up(&mut self, index: usize) -> usize {
        let mut index = index;
        unsafe {
            while index != 0 {
                let parent = (index - 1) / 2;
                {
                    let parent_node = self.data.get_unchecked(parent).node();
                    let index_node = self.data.get_unchecked(index).node();
                    if index_node.time >= parent_node.time {
                        break;
                    }
                }
                self.swap(index, parent);
                index = parent;
            }
        }
        index
    }

    fn sift_down(&mut self, index: usize) -> usize {
        let mut index = index;
        let end = self.data.len();
        unsafe {
            loop {
                let mut child = index * 2 + 1;
                if child >= end {
                    break;
                }
                {
                    let mut child_node: *const _ = self.data.get_unchecked(child).node();
                    let right = child + 1;
                    if right < end {
                        let right_node: *const _ = self.data.get_unchecked(right).node();
                        if (*right_node).time < (*child_node).time {
                            child = right;
                            child_node = right_node;
                        }
                    }
                    
                    let index_node = self.data.get_unchecked(index).node();
                    if index_node.time < (*child_node).time {
                        break;
                    }
                }
                self.swap(index, child);
                index = child;
            }
        }
        index
    }

    unsafe fn swap(&mut self, a: usize, b: usize) {
        let pa: *mut _ = self.data.get_unchecked_mut(a);
        let pb: *mut _ = self.data.get_unchecked_mut(b);
        ptr::swap(pa, pb);
        (*pa).node().index.set(a);
        (*pb).node().index.set(b);
    }
}

