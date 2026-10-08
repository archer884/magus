//! Starts a real server and has two bots play complete games over TCP.

use std::sync::Arc;
use std::time::Duration;

use magus_bot::Outcome;
use magus_core::CardPool;
use tokio::net::TcpListener;

#[tokio::test]
async fn two_bots_play_complete_games() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(magus_server::serve(listener, Arc::new(CardPool::builtin())));

    let matchups = [
        ("ember-thorn", "tide-ash"),
        ("dawn-grove", "storm-cinder"),
        ("tide-ash", "dawn-grove"),
    ];
    for (i, (deck_a, deck_b)) in matchups.into_iter().enumerate() {
        let room = format!("room-{i}");
        let a = tokio::spawn({
            let (addr, room) = (addr.clone(), room.clone());
            async move { magus_bot::run(&addr, "Alpha", &room, deck_a, Duration::ZERO).await }
        });
        // Make sure Alpha is waiting before Beta joins.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let b = tokio::spawn({
            let addr = addr.clone();
            async move { magus_bot::run(&addr, "Beta", &room, deck_b, Duration::ZERO).await }
        });
        let (a, b) = tokio::time::timeout(Duration::from_secs(60), async { (a.await, b.await) })
            .await
            .expect("game finished in time");
        let a = a.unwrap().unwrap().expect("alpha saw the end");
        let b = b.unwrap().unwrap().expect("beta saw the end");
        let consistent = matches!(
            (a, b),
            (Outcome::Won, Outcome::Lost)
                | (Outcome::Lost, Outcome::Won)
                | (Outcome::Draw, Outcome::Draw)
        );
        assert!(consistent, "{deck_a} vs {deck_b}: {a:?} / {b:?}");
    }
}
