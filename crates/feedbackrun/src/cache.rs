//! Session checkpoint cache. Host calls are deliberately kept outside the lock.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use after_effects as ae;
use feedbackrun_engine::{FeedbackStep, FrameBuffer};

const BUDGET_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RenderKey {
    pub instance: i32,
    pub origin: i32,
    pub depth: i16,
    pub time_step: i32,
    pub time_scale: u32,
    pub downsample_x: ae::RationalScale,
    pub downsample_y: ae::RationalScale,
    pub rect_x: i32,
    pub rect_y: i32,
}

impl RenderKey {
    pub fn from_in_data(in_data: &ae::InData, origin: i32, depth: i16) -> Self {
        Self {
            instance: in_data.effect().filter_instance_id().unwrap_or(0),
            origin,
            depth,
            time_step: in_data.time_step(),
            time_scale: in_data.time_scale(),
            downsample_x: in_data.downsample_x(),
            downsample_y: in_data.downsample_y(),
            rect_x: 0,
            rect_y: 0,
        }
    }
}

#[derive(Clone)]
pub struct Entry {
    pub key: RenderKey,
    pub frame: i32,
    pub receipt: ae::sys::PF_State,
    pub state: Arc<FrameBuffer>,
    serial: u64,
    used: u64,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    bytes: usize,
    clock: u64,
    pair_generations: HashMap<i32, u64>,
    pair_anchors: HashMap<i32, i32>,
    pair_configs: HashMap<(i32, i32), FeedbackConfig>,
    config_serials: HashMap<(i32, i32), u64>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct FeedbackConfig {
    pub step: FeedbackStep,
    pub delay_frames: i32,
    pub history_frames: i32,
    pub enabled: bool,
    pub dry_wet: f32,
    pub clear_to_current: bool,
}

#[derive(Clone, Copy)]
pub struct FrameConfig {
    pub anchor: i32,
    pub feedback: FeedbackConfig,
    pub serial: u64,
}

#[derive(Default)]
pub struct SessionCache(Mutex<Inner>);

impl SessionCache {
    pub fn anchor(&self, pair_id: i32) -> Option<i32> {
        self.0.lock().ok()?.pair_anchors.get(&pair_id).copied()
    }

    pub fn set_anchor(&self, pair_id: i32, anchor: i32) {
        if self.anchor(pair_id).is_some_and(|old| old != anchor) {
            self.reset_pair(pair_id);
        }
        if let Ok(mut cache) = self.0.lock() {
            cache.pair_anchors.insert(pair_id, anchor);
        }
    }

    /// Publish Copy's settings atomically before requesting Paste. A new
    /// config token invalidates any Paste frame rendered as pass-through.
    pub fn publish_configs(&self, pair_id: i32, configs: &[(i32, FeedbackConfig)]) {
        let Ok(mut cache) = self.0.lock() else { return };
        let changed_existing = configs.iter().any(|(frame, config)| {
            cache
                .pair_configs
                .get(&(pair_id, *frame))
                .is_some_and(|old| old != config)
        });
        if changed_existing {
            let mut removed_bytes = 0;
            cache.entries.retain(|entry| {
                if entry.key.instance == -pair_id {
                    removed_bytes += entry.state.byte_len();
                    false
                } else {
                    true
                }
            });
            cache.bytes -= removed_bytes;
            let generation = cache.pair_generations.entry(pair_id).or_default();
            *generation = generation.wrapping_add(1);
        }
        for &(frame, config) in configs {
            if cache.pair_configs.get(&(pair_id, frame)) != Some(&config) {
                cache.clock = cache.clock.wrapping_add(1);
                let serial = cache.clock;
                cache.pair_configs.insert((pair_id, frame), config);
                cache.config_serials.insert((pair_id, frame), serial);
            }
        }
    }

    pub fn frame_config(&self, pair_id: i32, frame: i32) -> Option<FrameConfig> {
        let cache = self.0.lock().ok()?;
        let anchor = *cache.pair_anchors.get(&pair_id)?;
        if frame < anchor {
            return None;
        }
        Some(FrameConfig {
            anchor,
            feedback: *cache.pair_configs.get(&(pair_id, frame))?,
            serial: *cache.config_serials.get(&(pair_id, frame))?,
        })
    }

    /// Changes only when the exact frame is replaced, not on cache reads.
    pub fn exact_serial(&self, key: RenderKey, frame: i32, width: usize, height: usize) -> u64 {
        self.0
            .lock()
            .ok()
            .and_then(|cache| {
                cache
                    .entries
                    .iter()
                    .find(|entry| {
                        entry.key == key
                            && entry.frame == frame
                            && entry.state.width() == width
                            && entry.state.height() == height
                    })
                    .map(|entry| entry.serial)
            })
            .unwrap_or(0)
    }

    pub fn pair_generation(&self, pair_id: i32) -> u64 {
        self.0
            .lock()
            .ok()
            .and_then(|cache| cache.pair_generations.get(&pair_id).copied())
            .unwrap_or(0)
    }

    /// Drop a pair's checkpoints and change the token mixed into AE's frame cache.
    pub fn reset_pair(&self, pair_id: i32) {
        let Ok(mut cache) = self.0.lock() else { return };
        let instance = -pair_id;
        let mut removed_bytes = 0;
        cache.entries.retain(|entry| {
            if entry.key.instance == instance {
                removed_bytes += entry.state.byte_len();
                false
            } else {
                true
            }
        });
        cache.bytes -= removed_bytes;
        cache.pair_configs.retain(|(id, _), _| *id != pair_id);
        cache.config_serials.retain(|(id, _), _| *id != pair_id);
        let generation = cache.pair_generations.entry(pair_id).or_default();
        *generation = generation.wrapping_add(1);
    }

    pub fn candidates(&self, key: RenderKey, end: i32) -> Vec<Entry> {
        let Ok(cache) = self.0.lock() else {
            return Vec::new();
        };
        let mut found: Vec<_> = cache
            .entries
            .iter()
            .filter(|entry| entry.key == key && entry.frame <= end)
            .cloned()
            .collect();
        found.sort_by_key(|entry| std::cmp::Reverse(entry.frame));
        found
    }

    pub fn insert(&self, mut entry: Entry) {
        let Ok(mut cache) = self.0.lock() else { return };
        cache.clock = cache.clock.wrapping_add(1);
        entry.used = cache.clock;
        entry.serial = cache.clock;
        let mut removed_bytes = 0;
        cache.entries.retain(|old| {
            let replace = old.key == entry.key && old.frame == entry.frame;
            if replace {
                removed_bytes += old.state.byte_len();
            }
            !replace
        });
        cache.bytes -= removed_bytes;
        let size = entry.state.byte_len();
        if size > BUDGET_BYTES {
            return;
        }
        cache.bytes += size;
        cache.entries.push(entry);
        while cache.bytes > BUDGET_BYTES {
            let Some((index, _)) = cache.entries.iter().enumerate().min_by_key(|(_, e)| e.used)
            else {
                break;
            };
            let removed = cache.entries.swap_remove(index);
            cache.bytes -= removed.state.byte_len();
        }
    }

    pub fn touch(&self, key: RenderKey, frame: i32) {
        let Ok(mut cache) = self.0.lock() else { return };
        cache.clock = cache.clock.wrapping_add(1);
        let used = cache.clock;
        if let Some(entry) = cache
            .entries
            .iter_mut()
            .find(|e| e.key == key && e.frame == frame)
        {
            entry.used = used;
        }
    }
}

impl Entry {
    pub fn new(
        key: RenderKey,
        frame: i32,
        receipt: ae::sys::PF_State,
        state: Arc<FrameBuffer>,
    ) -> Self {
        Self {
            key,
            frame,
            receipt,
            state,
            serial: 0,
            used: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_checkpoint_writes_remain_available_in_order() {
        let cache = Arc::new(SessionCache::default());
        let key = RenderKey {
            instance: 7,
            origin: 0,
            depth: 32,
            time_step: 1,
            time_scale: 24,
            downsample_x: ae::RationalScale { num: 1, den: 1 },
            downsample_y: ae::RationalScale { num: 1, den: 1 },
            rect_x: 0,
            rect_y: 0,
        };
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let cache = Arc::clone(&cache);
                scope.spawn(move || {
                    for i in 0..32 {
                        let frame = worker * 32 + i;
                        cache.insert(Entry::new(
                            key,
                            frame,
                            ae::sys::PF_State { reservedAL: [0; 4] },
                            Arc::new(FrameBuffer::transparent(1, 1)),
                        ));
                        cache.touch(key, frame);
                    }
                });
            }
        });
        let candidates = cache.candidates(key, 200);
        assert_eq!(candidates.len(), 201);
        assert_eq!(candidates[0].frame, 200);
        assert_eq!(candidates[200].frame, 0);
    }
}
