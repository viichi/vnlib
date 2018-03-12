
use std::rc::Rc;
use std::cell::{RefCell, Cell};
use std::time::{Instant, Duration};

use super::core::{TimedAction, TimedActionNode};

pub struct Timer {
    data: Rc<Data>,
    cancel_on_drop: Cell<bool>,
}

impl Timer {
    pub fn new() -> Self {
        Timer {
            data: Rc::new(Data {
                n: TimedActionNode::new(),
                i: RefCell::new(Inner {
                    state: State::None,
                    act: None,
                }),
            }),
            cancel_on_drop: Cell::new(false),
        }
    }

    pub fn with_callback<T>(self, cb: T) -> Self where T: FnMut() + 'static {
        self.set_callback(cb);
        self
    }

    pub fn with_callback_once<T>(self, cb: T) -> Self where T: FnOnce() + 'static {
        self.set_callback_once(cb);
        self
    }

    pub fn with_cancel_on_drop(self, cancel_on_drop: bool) -> Self {
        self.cancel_on_drop.set(cancel_on_drop);
        self
    }

    pub fn and_start(self, time: Duration) -> Self {
        self.start(time);
        self
    }

    pub fn set_callback<T>(&self, cb: T) where T: FnMut() + 'static {
        let mut inner = self.data.i.borrow_mut();
        inner.act = Some(Box::new(cb));
    }

    pub fn set_callback_once<T>(&self, cb: T) where T: FnOnce() + 'static {
        let mut inner = self.data.i.borrow_mut();
        inner.act = Some(Box::new(Some(cb)));
    }

    pub fn is_active(&self) -> bool {
        match self.data.i.borrow().state {
            State::Active | State::Restart(_) => true,
            _ => false,
        }
    }

    pub fn set_cancel_on_drop(&self, cancel_on_drop: bool) {
        self.cancel_on_drop.set(cancel_on_drop);
    }

    pub fn is_cancel_on_drop(&self) -> bool {
        self.cancel_on_drop.get()
    }

    pub fn start(&self, time: Duration) {
        let mut inner = self.data.i.borrow_mut();
        match inner.state {
            State::None => {
                super::push_timed_action(self.data.clone(), Instant::now() + time);
            },
            State::Active => {
                super::adjust_timed_action(&self.data.n, Instant::now() + time);
            },
            State::Processing | State::Restart(_) => {
                inner.state = State::Restart(Instant::now() + time);
            },
        }
    }

    pub fn cancel(&self) {
        let mut inner = self.data.i.borrow_mut();
        match inner.state {
            State::None | State::Processing => {},
            State::Active => {
                super::remove_timed_action(&self.data.n);
            },
            State::Restart(_) => {
                inner.state = State::Processing;
            },
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if self.cancel_on_drop.get() {
            self.cancel();
        }
    }
}



struct Data {
    n: TimedActionNode,
    i: RefCell<Inner>,
}

struct Inner {
    state: State,
    act: Option<Box<Action>>,
}

enum State {
    None,
    Active,
    Processing,
    Restart(Instant),
}

impl TimedAction for Data {
    fn node(&self) -> &TimedActionNode {
        &self.n
    }

    fn process(&self) -> Option<Instant> {
        let mut inner = self.i.borrow_mut();
        if let Some(mut f) = inner.act.take() {
            inner.state = State::Processing;
            drop(inner);
            let ok = f.call();
            inner = self.i.borrow_mut();
            if inner.act.is_none() && ok {
                inner.act = Some(f);
            }
            match inner.state {
                State::Processing => {
                    inner.state = State::None;
                    None
                },
                State::Restart(t) => {
                    inner.state = State::Active;
                    Some(t)
                },
                _ => unreachable!(),
            }
        }
        else {
            inner.state = State::None;
            None
        }
    }
}

trait Action {
    fn call(&mut self) -> bool;
}

impl<T> Action for T where T: FnMut() {
    fn call(&mut self) -> bool {
        self();
        true
    }
}

impl<T> Action for Option<T> where T: FnOnce() {
    fn call(&mut self) -> bool {
        if let Some(t) = self.take() {
            t();
        }
        false
    }
}

/*
struct ActionOnce<T> {
    once: Option<T>,
}

impl<T> Action for ActionOnce<T> where T: FnOnce() {
    fn call(&mut self) -> bool {
        if let Some(f) = self.once.take() {
            f();
        }
        false
    }
}
*/