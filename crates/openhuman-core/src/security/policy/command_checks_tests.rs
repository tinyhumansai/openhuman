use super::*;

#[test]
fn scan_guard_allows_targeted_project_and_sdk_paths_and_catches_default_recursive_rg() {
    for cmd in [
        "find /private/tmp/project -name '*.cpp'",
        "find /Library/Developer/CommandLineTools/SDKs -name 'stdio.h'",
        "ls /Library/Developer",
        "find /usr/include -name 'stddef.h'",
        "find . -name '*.cpp'",
        "rg /Applications src",
    ] {
        assert!(is_unbounded_system_scan(cmd).is_none(), "{cmd}");
    }
    for cmd in [
        "rg pattern /",
        "find -L /System -name '*.h'",
        "find -O3 /System -name '*.h'",
        "find /Applications/. -name secret",
        "ls -R /Library",
        "rg --files /Applications",
    ] {
        assert!(is_unbounded_system_scan(cmd).is_some(), "{cmd}");
    }
}
