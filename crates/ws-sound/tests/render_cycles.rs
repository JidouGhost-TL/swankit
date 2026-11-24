//! Cycle-timed rendering on a synthetic ramp wave (sample = index & 15).
use ws_sound::render::{render_cycles, Action, CounterReload, CycleEvent};
use ws_sound::WaveBank;

fn ramp() -> WaveBank {
    let mut bytes = [0u8; 64];
    for (k, byte) in bytes.iter_mut().take(16).enumerate() {
        *byte = ((2 * k as u8) & 15) | (((2 * k as u8 + 1) & 15) << 4);
    }
    WaveBank::decode(&bytes).unwrap()
}

fn events(change_at: u64) -> Vec<CycleEvent> {
    let w = |cycle, port, value| CycleEvent {
        cycle,
        action: Action::Write { port, value },
    };
    let divisor = 2048 - 100u16;
    let slow = 2048 - 1000u16;
    vec![
        CycleEvent {
            cycle: 0,
            action: Action::Waves(ramp()),
        },
        w(0, 0x91, 0x08),
        w(0, 0x88, 0x11),
        w(0, 0x80, divisor as u8),
        w(0, 0x81, (divisor >> 8) as u8),
        w(0, 0x90, 0x01),
        w(change_at, 0x80, slow as u8),
        w(change_at, 0x81, (slow >> 8) as u8),
    ]
}

fn levels(reload: CounterReload) -> Vec<i16> {
    let pcm = render_cycles(&events(250), 12, reload).unwrap();
    pcm.samples().iter().step_by(2).map(|s| s / 32).collect()
}

#[test]
fn steps_follow_exact_cycles_and_reload_rule() {
    // Period 100 from cycle 0: steps at 100 and 200 before the change at 250.
    // Remainder: 50 elapsed cycles carry into the 1000-cycle period (step at 1200).
    let remainder = levels(CounterReload::Remainder);
    assert_eq!(remainder[..3], [1, 2, 2]);
    assert_eq!(remainder[8], 2); // frame 8 ends at cycle 1152
    assert_eq!(remainder[9], 3); // frame 9 ends at cycle 1280
                                 // NextPeriod: the running 100-cycle period ends at 300, then 1000 cycles.
    let next = levels(CounterReload::NextPeriod);
    assert_eq!(next[..3], [1, 2, 3]);
    assert_eq!(next[9], 3);
    assert_eq!(next[10], 4); // frame 10 ends at cycle 1408 (step at 1300)
}

#[test]
fn unordered_events_are_rejected() {
    let mut list = events(250);
    list.swap(0, 7);
    assert!(render_cycles(&list, 4, CounterReload::Remainder).is_err());
}
