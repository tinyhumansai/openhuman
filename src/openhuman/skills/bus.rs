//! Legacy no-op event bus hooks retained while call-sites are cleaned up.

pub fn register_skill_cleanup_subscriber() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_skill_cleanup_subscriber_is_a_safe_noop() {
        // The function is intentionally empty while call-sites migrate
        // off the legacy bus hook — calling it repeatedly must remain
        // side-effect free.
        register_skill_cleanup_subscriber();
        register_skill_cleanup_subscriber();
    }
}
