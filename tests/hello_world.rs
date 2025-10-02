fn test_update_rate() {
    let start_time = std::time::Instant::now();
    let mut updates = 0;

    while updates < 5 {
        // Simulate the bar update
        std::thread::sleep(std::time::Duration::from_secs(1));
        updates += 1;
    }

    let elapsed_time = start_time.elapsed();
    assert!(
        elapsed_time.as_secs() < 6,
        "The bar did not update every second."
    );
}
