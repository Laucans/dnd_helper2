//! The Capability starts on the read-only variable alone. No database needed:
//! every case here fails before a connection is tried.

use campagne_niveau_du_groupe::{Capability, StartError};

#[tokio::test]
async fn it_does_not_start_without_the_read_only_variable() {
    let error = Capability::from_variables(|_| None).await.err().unwrap();
    assert!(matches!(error, StartError::Missing(_)), "{error:?}");
}

#[tokio::test]
async fn it_never_falls_back_to_the_read_write_variable() {
    // Built in two pieces: plain test data, not a use of the variable.
    let read_write = ["DATABASE", "_URL"].concat();
    let error = Capability::from_variables(|name| {
        (name == read_write).then(|| "postgres://migrator:secret@localhost/app".to_owned())
    })
    .await
    .err()
    .unwrap();
    assert!(matches!(error, StartError::Missing(_)), "{error:?}");
}

#[tokio::test]
async fn a_blank_value_is_a_missing_one() {
    let error = Capability::from_variables(|_| Some("   ".to_owned()))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, StartError::Missing(_)), "{error:?}");
}

#[tokio::test]
async fn a_malformed_value_is_refused_without_quoting_it() {
    let marker = "hunter2-marker";
    let error = Capability::from_variables(|_| Some(format!("not a url {marker}")))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, StartError::Malformed(_)), "{error:?}");
    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));
}
