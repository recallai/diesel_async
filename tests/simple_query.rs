use diesel::sql_types::Integer;
use diesel::IntoSql;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use tokio_postgres::SimpleQueryMessage;

fn rows(messages: Vec<SimpleQueryMessage>) -> Vec<tokio_postgres::SimpleQueryRow> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn simple_queries_reuse_connection_without_preparing_and_interleave_with_diesel() {
    let mut con = AsyncPgConnection::establish(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let sql = "SELECT pg_backend_pid() AS pid, (SELECT count(*) FROM pg_prepared_statements) AS prepared, clock_timestamp()::text AS clock";
    let first = rows(con.simple_query(sql).await.unwrap());
    assert_eq!(first[0].get("prepared"), Some("0"));
    let again = rows(con.simple_query(sql).await.unwrap());
    assert_eq!(again[0].get("pid"), first[0].get("pid"));
    assert_ne!(again[0].get("clock"), first[0].get("clock"));
    assert_eq!(again[0].get("prepared"), Some("0"));

    for value in [1, 2] {
        assert_eq!(
            diesel::select(value.into_sql::<Integer>())
                .get_result::<i32>(&mut con)
                .await
                .unwrap(),
            value
        );
        let mixed = rows(con.simple_query(sql).await.unwrap());
        assert_eq!(mixed[0].get("pid"), first[0].get("pid"));
        assert_eq!(mixed[0].get("prepared"), Some("1"));
    }
}

#[tokio::test]
async fn simple_query_errors_rollback_and_connection_remains_usable() {
    use diesel::result::{DatabaseErrorKind, Error};
    let mut con = AsyncPgConnection::establish(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let err = con.transaction::<(), Error, _>(|con| Box::pin(async move {
        con.simple_query("DO $$ BEGIN RAISE EXCEPTION 'test serialization failure' USING ERRCODE = '40001'; END $$").await?;
        Ok(())
    })).await.unwrap_err();
    assert!(matches!(
        err,
        Error::DatabaseError(DatabaseErrorKind::SerializationFailure, _)
    ));
    assert_eq!(
        rows(con.simple_query("SELECT 42 AS value").await.unwrap())[0].get("value"),
        Some("42")
    );
}
