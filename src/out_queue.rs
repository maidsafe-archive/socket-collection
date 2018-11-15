// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;
use {Priority, SocketConfig};

/// Socket outgoing message queue with priorities and message expiration.
#[derive(Default)]
pub struct OutQueue {
    inner: BTreeMap<Priority, VecDeque<(Instant, Vec<u8>)>>,
    conf: SocketConfig,
}

impl OutQueue {
    /// Constructs empty write queue.
    pub fn new(conf: SocketConfig) -> Self {
        Self {
            inner: Default::default(),
            conf,
        }
    }

    /// Push buffer to the queue with the given priority.
    pub fn push(&mut self, buf: Vec<u8>, priority: Priority) {
        self.push_at(Instant::now(), buf, priority);
    }

    /// Drop expired messages.
    pub fn drop_expired(&mut self) -> usize {
        let expired_keys = self.expired_queues();
        let dropped_msgs: usize = expired_keys
            .iter()
            .filter_map(|priority| self.inner.remove(priority))
            .map(|queue| queue.len())
            .sum();
        if dropped_msgs > 0 {
            trace!(
                "Insufficient bandwidth. Dropping {} messages with priority >= {}.",
                dropped_msgs,
                expired_keys[0]
            );
        }
        dropped_msgs
    }

    /// Returns next outgoing message. Messages with lower priority number are first.
    pub fn next_msg(&mut self) -> Option<Vec<u8>> {
        let (key, (_time_stamp, data), empty) = match self.inner.iter_mut().next() {
            Some((key, queue)) => (*key, unwrap!(queue.pop_front()), queue.is_empty()),
            None => return None,
        };
        if empty {
            let _ = self.inner.remove(&key);
        }
        Some(data)
    }

    /// Helper method for testing that is able to push buffer to outgoing queue with a given
    /// timestamp. Helps testing expired messages.
    fn push_at(&mut self, when: Instant, buf: Vec<u8>, priority: Priority) {
        let entry = self
            .inner
            .entry(priority)
            .or_insert_with(|| VecDeque::with_capacity(10));
        entry.push_back((when, buf));
    }

    /// Returns a list of out queues with expired messages or no messages at all.
    fn expired_queues(&self) -> Vec<u8> {
        self.inner
            .iter()
            .skip_while(|&(&priority, queue)| is_queue_valid(priority, queue, &self.conf))
            .map(|(&priority, _)| priority)
            .collect()
    }
}

/// Checks if given message queue should not be dropped.
fn is_queue_valid(priority: u8, queue: &VecDeque<(Instant, Vec<u8>)>, conf: &SocketConfig) -> bool {
    priority < conf.msg_drop_priority || // Don't drop high-priority messages.
    queue.front().map_or(true, |&(ref timestamp, _)| {
        timestamp.elapsed().as_secs() <= conf.max_msg_age_secs
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    mod is_queue_valid {
        use super::*;

        #[test]
        fn it_returns_true_when_queue_priority_is_smaller_than_minimum_drop_priority() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 2;
            let queue = VecDeque::new();

            let retain = is_queue_valid(1, &queue, &conf);

            assert!(retain);
        }

        mod when_queue_priority_is_dropable {
            use super::*;
            use std::ops::Sub;

            #[test]
            fn it_returns_true_when_queue_is_empty() {
                let mut conf = SocketConfig::default();
                conf.msg_drop_priority = 2;
                let queue = VecDeque::new();

                let retain = is_queue_valid(2, &queue, &conf);

                assert!(retain);
            }

            #[test]
            fn it_returns_false_when_first_queue_is_expired() {
                let mut conf = SocketConfig::default();
                conf.msg_drop_priority = 2;
                conf.max_msg_age_secs = 10;
                let mut queue = VecDeque::new();
                let queued_at = Instant::now().sub(Duration::from_secs(100));
                queue.push_back((queued_at, vec![1, 2, 3]));

                let retain = is_queue_valid(2, &queue, &conf);

                assert!(!retain);
            }

            #[test]
            fn it_returns_true_when_first_queue_item_is_not_expired() {
                let mut conf = SocketConfig::default();
                conf.msg_drop_priority = 2;
                conf.max_msg_age_secs = 10;
                let mut queue = VecDeque::new();
                let queued_at = Instant::now().sub(Duration::from_secs(5));
                queue.push_back((queued_at, vec![1, 2, 3]));

                let retain = is_queue_valid(2, &queue, &conf);

                assert!(retain);
            }
        }
    }

    mod expired_queues {
        use super::*;
        use std::ops::Sub;

        #[test]
        fn it_returns_list_of_expired_queues() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 1;
            conf.max_msg_age_secs = 10;

            let mut out_queue = OutQueue::new(conf);

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let queued_at = Instant::now().sub(Duration::from_secs(100));
            out_queue.push_at(queued_at, vec![1, 2, 3], 2);

            let queued_at = Instant::now().sub(Duration::from_secs(200));
            out_queue.push_at(queued_at, vec![1, 2, 3], 3);

            let expired = out_queue.expired_queues();

            assert_eq!(expired, vec![2, 3]);
        }

        #[test]
        fn when_first_queue_is_expired_it_wont_check_any_further() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 1;
            conf.max_msg_age_secs = 10;

            let mut out_queue = OutQueue::new(conf);

            let queued_at = Instant::now().sub(Duration::from_secs(100));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 2);

            let queued_at = Instant::now().sub(Duration::from_secs(6));
            out_queue.push_at(queued_at, vec![1, 2, 3], 3);

            let queued_at = Instant::now().sub(Duration::from_secs(200));
            out_queue.push_at(queued_at, vec![1, 2, 3], 4);

            let expired = out_queue.expired_queues();

            // NOTE(povilas): this is unexpected behavior to me. I expected it to return [1, 4].
            // Such behavior is required by Routing and might be changed in the future.
            assert_eq!(expired, vec![1, 2, 3, 4]);
        }
    }

    mod drop_expired {
        use super::*;
        use std::ops::Sub;

        #[test]
        fn it_drops_queues_with_expired_messages() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 1;
            conf.max_msg_age_secs = 10;
            let mut out_queue = OutQueue::new(conf);

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let queued_at = Instant::now().sub(Duration::from_secs(100));
            out_queue.push_at(queued_at, vec![4, 5, 6], 2);

            let queued_at = Instant::now().sub(Duration::from_secs(200));
            out_queue.push_at(queued_at, vec![7, 8, 9], 3);

            let dropped = out_queue.drop_expired();

            assert_eq!(dropped, 2);
            assert_eq!(out_queue.next_msg(), Some(vec![1, 2, 3]));
        }

        #[test]
        fn it_does_not_drop_expired_message_whose_priority_is_lower_than_drop_priority() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 2;
            conf.max_msg_age_secs = 10;
            let mut out_queue = OutQueue::new(conf);

            let queued_at = Instant::now().sub(Duration::from_secs(100));
            out_queue.push_at(queued_at, vec![1, 2, 3], 2);

            let queued_at = Instant::now().sub(Duration::from_secs(200));
            out_queue.push_at(queued_at, vec![3, 4, 5], 1);

            let dropped = out_queue.drop_expired();

            assert_eq!(dropped, 1);
            assert_eq!(out_queue.next_msg(), Some(vec![3, 4, 5]));
        }
    }

    mod next_msg {
        use super::*;
        use std::ops::Sub;

        #[test]
        fn it_returns_none_if_no_data_is_queued() {
            let mut out_queue = OutQueue::new(SocketConfig::default());

            let next_msg = out_queue.next_msg();

            assert!(next_msg.is_none());
        }

        #[test]
        fn it_returns_next_message_when_data_is_queued() {
            let mut out_queue = OutQueue::new(SocketConfig::default());

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let next_msg = out_queue.next_msg();

            assert_eq!(next_msg, Some(vec![1, 2, 3]));
        }

        #[test]
        fn it_returns_next_message_from_lower_priority_queue() {
            let mut out_queue = OutQueue::new(SocketConfig::default());

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 2);

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let next_msg = out_queue.next_msg();

            assert_eq!(next_msg, Some(vec![1, 2, 3]));
        }

        #[test]
        fn it_removes_queue_if_it_had_only_1_element() {
            let mut conf = SocketConfig::default();
            conf.msg_drop_priority = 1;
            let mut out_queue = OutQueue::new(conf);

            let queued_at = Instant::now().sub(Duration::from_secs(5));
            out_queue.push_at(queued_at, vec![1, 2, 3], 1);

            let _ = out_queue.next_msg();

            assert_eq!(out_queue.inner.len(), 0);
        }
    }
}
