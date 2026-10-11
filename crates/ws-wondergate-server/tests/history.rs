// SPDX-License-Identifier: MIT OR Apache-2.0
#[path = "../examples/support/history.rs"]
mod history;
use history::{FixtureStore, MAX_HISTORY};
use ws_wondergate_server::{
    board::{Board, Order},
    store::{Scope, Store, Submission, MAX_IDENTITY_SUBMISSIONS, MAX_SUBMISSIONS},
    Timestamp,
};
fn row(identity: &[u8], score: u64) -> Submission {
    Submission {
        scope: Scope::new("fixture", "board", "1"),
        identity: identity.to_vec(),
        name: b"PLAYER".to_vec(),
        score,
        extra: 0,
        submitted_at: Timestamp::parse_fixed("20261010120000").unwrap(),
    }
}
#[test]
fn frozen_history_can_exceed_live_capacity_without_consuming_new_write_quota() {
    let history = (0..=MAX_SUBMISSIONS)
        .map(|n| row(&n.to_le_bytes(), n as u64))
        .collect();
    let store = FixtureStore::new(history).unwrap();
    let scope = row(b"LIVE", 1).scope;
    assert_eq!(
        store.submissions(&scope).unwrap().len(),
        MAX_SUBMISSIONS + 1
    );
    assert_eq!(
        store.submit(row(b"LIVE", 1)).unwrap(),
        (MAX_SUBMISSIONS + 2) as u64
    );
    assert_eq!(
        store.submissions(&scope).unwrap().len(),
        MAX_SUBMISSIONS + 2
    );
}
#[test]
fn fixture_writes_keep_live_identity_quota_and_historical_tie_order() {
    let store = FixtureStore::new(vec![row(b"ALPHA", 10)]).unwrap();
    for _ in 0..MAX_IDENTITY_SUBMISSIONS {
        store.submit(row(b"ALPHA", 10)).unwrap();
    }
    assert!(store.submit(row(b"ALPHA", 11)).is_err());
    store.submit(row(b"BETA", 10)).unwrap();
    let leaders = Board::new(&store, row(b"ALPHA", 10).scope, Order::Descending)
        .leaders(2)
        .unwrap();
    assert_eq!(leaders[0].submission.identity, b"ALPHA");
    assert_eq!(leaders[1].submission.identity, b"BETA");
    assert_eq!(leaders[0].id, 1);
    assert_eq!(leaders[1].id, (MAX_IDENTITY_SUBMISSIONS + 2) as u64);
}
#[test]
fn fixture_history_rejects_oversized_or_invalid_rows() {
    assert!(FixtureStore::new(vec![row(b"X", 1); MAX_HISTORY + 1]).is_err());
    let mut invalid = row(b"X", 1);
    invalid.scope.definition = "invalid/scope".into();
    assert!(FixtureStore::new(vec![invalid]).is_err());
    assert!(FixtureStore::new(vec![row(&[0; 513], 1)]).is_err());
}
