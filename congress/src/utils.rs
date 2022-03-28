use std::time::Duration;

use rand::Rng;

pub fn get_random_timeout() -> Duration {
    let mut rng = rand::thread_rng();
    let timeout = rng.gen_range(400..800);
    Duration::from_millis(timeout)
}
