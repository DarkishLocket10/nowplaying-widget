fn test_bar_update_rate() {
    let update_interval = 1; // expected update interval in seconds
    let mut last_update_time = std::time::Instant::now();

    // Simulate the bar updating
    loop {
        let current_time = std::time::Instant::now();
        if current_time.duration_since(last_update_time).as_secs() >= update_interval {
            last_update_time = current_time;
            break; // exit loop after simulating one update
        }
    }

    assert!(last_update_time.elapsed().as_secs() == update_interval);
}
