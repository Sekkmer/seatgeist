use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

#[derive(Debug)]
struct State<T> {
    pending: Option<T>,
    sender_count: usize,
}

#[derive(Debug)]
struct Shared<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

#[derive(Debug)]
pub(crate) struct LatestSender<T> {
    shared: Arc<Shared<T>>,
}

#[derive(Debug)]
pub(crate) struct LatestReceiver<T> {
    shared: Arc<Shared<T>>,
}

pub(crate) fn channel<T>() -> (LatestSender<T>, LatestReceiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            pending: None,
            sender_count: 1,
        }),
        changed: Condvar::new(),
    });
    (
        LatestSender {
            shared: Arc::clone(&shared),
        },
        LatestReceiver { shared },
    )
}

impl<T> LatestSender<T> {
    /// Replace any unread value. Capture callers care about the newest frame,
    /// not every intermediate frame produced while no snapshot was requested.
    pub(crate) fn send(&self, value: T) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        state.pending = Some(value);
        self.shared.changed.notify_one();
    }
}

impl<T> Clone for LatestSender<T> {
    fn clone(&self) -> Self {
        if let Ok(mut state) = self.shared.state.lock() {
            state.sender_count = state.sender_count.saturating_add(1);
        }
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for LatestSender<T> {
    fn drop(&mut self) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        state.sender_count = state.sender_count.saturating_sub(1);
        if state.sender_count == 0 {
            self.shared.changed.notify_all();
        }
    }
}

impl<T> LatestReceiver<T> {
    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
        let (mut state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.pending.is_none() && state.sender_count > 0
            })
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
        if let Some(value) = state.pending.take() {
            return Ok(value);
        }
        if state.sender_count == 0 {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        } else {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unread_values_collapse_to_the_latest() {
        let (sender, receiver) = channel();
        sender.send(1);
        sender.send(2);
        sender.send(3);

        assert_eq!(receiver.recv_timeout(Duration::from_millis(1)), Ok(3));
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(1)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
    }

    #[test]
    fn receiver_reports_disconnection_after_the_last_sender_drops() {
        let (sender, receiver) = channel::<u8>();
        let clone = sender.clone();
        drop(sender);
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(1)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        drop(clone);
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(1)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        );
    }
}
