
use std::rc::Rc;
use std::cell::{RefCell, Cell};
use std::time::{Instant, Duration};

use super::core::{TimedAction, TimedActionNode};

/// 周期历程
/// 
/// # Examples
/// ```
/// use vnbase::run_loop;
/// use std::time::Duration;
/// 
/// let mut time = Duration::default();
/// 
/// let _schedule = run_loop::new_schedule()
///     .with_period(Duration::from_millis(100))
///     .with_callback(move |dt| {
///         time += dt;
///         if time >= Duration::from_secs(1) {
///             run_loop::stop();
///         }
///     })
///     .and_start();
/// 
/// run_loop::run();
/// ```
pub struct Schedule {
    data: Rc<Data>,
    cancel_on_drop: Cell<bool>,
}

impl Schedule {
    pub fn new() -> Self {
        let now = Instant::now();
        Schedule {
            data: Rc::new(Data {
                n: TimedActionNode::new(),
                i: RefCell::new(Inner {
                    state: State::None,
                    period: Duration::from_millis(100),
                    last: now,
                    target: now,
                    act: None,
                }),
            }),
            cancel_on_drop: Cell::new(false),
        }
    }

    pub fn with_callback<T>(self, cb: T) -> Self where T: FnMut(Duration) + 'static {
        self.set_callback(cb);
        self
    }

    pub fn with_period(self, period: Duration) -> Self {
        self.set_period(period);
        self
    }

    pub fn with_cancel_on_drop(self, cancel_on_drop: bool) -> Self {
        self.cancel_on_drop.set(cancel_on_drop);
        self
    }

    pub fn and_start(self) -> Self {
        self.start();
        self
    }

    pub fn set_callback<T>(&self, cb: T) where T: FnMut(Duration) + 'static {
        let mut inner = self.data.i.borrow_mut();
        inner.act = Some(Box::new(cb));
    }

    pub fn set_period(&self, period: Duration) {
        let mut inner = self.data.i.borrow_mut();
        if inner.period != period {
            inner.period = period;
            if inner.state == State::Active {
                super::adjust_timed_action(&self.data.n, inner.last + period);
            }
        }
    }

    pub fn get_period(&self) -> Duration {
        self.data.i.borrow().period
    }

    pub fn set_cancel_on_drop(&self, cancel_on_drop: bool) {
        self.cancel_on_drop.set(cancel_on_drop);
    }

    pub fn is_cancel_on_drop(&self) -> bool {
        self.cancel_on_drop.get()
    }

    pub fn is_active(&self) -> bool {
        match self.data.i.borrow().state {
            State::None | State::Cancelled => false,
            _ => true,
        }
    }

    pub fn start(&self) {
        let mut inner = self.data.i.borrow_mut();
        let now = Instant::now();
        inner.last = now;
        inner.target = now + inner.period;
        match inner.state {
            State::None => {
                super::push_timed_action(self.data.clone(), inner.target);
            },
            State::Active => {
                super::adjust_timed_action(&self.data.n, inner.target);
            },
            State::Processing => {},
            State::Cancelled => {
                inner.state = State::Processing;
            },
        }
    }

    pub fn cancel(&self) {
        let mut inner = self.data.i.borrow_mut();
        match inner.state {
            State::None | State::Cancelled => {},
            State::Active => {
                super::remove_timed_action(&self.data.n);
            },
            State::Processing => {
                inner.state = State::Cancelled;
            },
        }
    }
}

impl Drop for Schedule {
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
    period: Duration,
    last: Instant,
    target: Instant,
    act: Option<Box<FnMut(Duration)>>,
}

#[derive(PartialEq, Eq)]
enum State {
    None,
    Active,
    Processing,
    Cancelled,
}

impl TimedAction for Data {
    fn node(&self) -> &TimedActionNode {
        &self.n
    }

    fn process(&self) -> Option<Instant> {
        let mut inner = self.i.borrow_mut();
        let now = Instant::now();
        let dur = now - inner.last;
        inner.last = now;
        inner.target += inner.period;
        if let Some(mut f) = inner.act.take() {
            inner.state = State::Processing;
            drop(inner);
            f(dur);
            inner = self.i.borrow_mut();
            if inner.act.is_none() {
                inner.act = Some(f);
            }
            match inner.state {
                State::Processing => {
                    inner.state = State::Active;
                    Some(inner.target)
                },
                State::Cancelled => {
                    inner.state = State::None;
                    None
                },
                _ => unreachable!(),
            }
        }
        else {
            Some(inner.target)
        }
    }
}