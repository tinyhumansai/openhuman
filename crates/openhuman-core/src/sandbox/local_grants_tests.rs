use super::*;

#[test]
fn saas_never_consults_the_operator_home() {
    let home = grant_home(true, || {
        panic!("a tenant must not read the operator's home")
    });
    assert!(home.is_none());
}
#[test]
fn single_user_keeps_the_operator_home() {
    let home = grant_home(false, || Some(PathBuf::from("/home/op")));
    assert_eq!(home, Some(PathBuf::from("/home/op")));
}
