// Smoke test: tty module is platform-specific. On Windows it's a no-op
// type; on Unix it manipulates the controlling terminal which we cannot
// safely modify in a test runner. So we only verify that the module loads
// and that the no-op enable() path returns Ok on Windows.

#[cfg(not(unix))]
#[test]
fn raw_tty_enable_is_noop_on_windows() {
    let r = claude_phone_wrapper::tty::RawTty::enable();
    assert!(r.is_ok(), "enable() on Windows must be a no-op success");
}

#[cfg(unix)]
#[test]
fn raw_tty_type_exists() {
    // We do NOT call enable() in tests on Unix — that would tcsetattr on
    // the test runner's stdin and corrupt CI output. Just assert the
    // type is accessible.
    fn _accepts<T>(_: &T) {}
    // Can't construct without enable(); use type-level check instead.
    let _ = std::marker::PhantomData::<claude_phone_wrapper::tty::RawTty>;
}
