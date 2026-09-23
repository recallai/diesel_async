use diesel::result::{DatabaseErrorKind, Error};
use diesel::sql_types::{Array, BigInt, Integer, Nullable, Text};
use diesel::{sql_query, IntoSql, QueryResult, QueryableByName};
use diesel_async::{AsyncConnection, RunQueryDsl};
use scoped_futures::ScopedFutureExt;

#[derive(Debug, PartialEq, QueryableByName)]
struct Values {
    #[diesel(sql_type = Array<Integer>)]
    numbers: Vec<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    label: Option<String>,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[tokio::test]
async fn raw_queries_use_unnamed_statements_and_rebind_values() -> QueryResult<()> {
    let conn = &mut super::connection().await;
    for (numbers, label) in [(vec![1, 2], Some("first")), (vec![3], None)] {
        let result: Values = sql_query("SELECT $1 AS numbers, $2 AS label")
            .bind::<Array<Integer>, _>(&numbers)
            .bind::<Nullable<Text>, _>(label)
            .get_result(conn)
            .await?;
        assert_eq!(result.numbers, numbers);
        assert_eq!(result.label.as_deref(), label);
    }

    // Check while this query itself is executing: preparing and subsequently
    // closing a named statement would leave this query visible here.
    let count: Count = sql_query("SELECT count(*)::bigint AS count FROM pg_prepared_statements")
        .get_result(conn)
        .await?;
    assert_eq!(count.count, 0);

    // Cacheable Diesel DSL queries must still retain their named statements.
    diesel::select(1_i32.into_sql::<Integer>())
        .get_result::<i32>(conn)
        .await?;
    let count: Count = sql_query("SELECT count(*)::bigint AS count FROM pg_prepared_statements")
        .get_result(conn)
        .await?;
    assert_eq!(count.count, 1);
    Ok(())
}

#[tokio::test]
async fn raw_execute_handles_no_rows_returning_and_errors() -> QueryResult<()> {
    let conn = &mut super::connection().await;
    assert_eq!(
        sql_query("INSERT INTO users (name) VALUES ($1), ($2)")
            .bind::<Text, _>("first")
            .bind::<Text, _>("second")
            .execute(conn)
            .await?,
        2
    );
    assert_eq!(
        sql_query("UPDATE users SET name = $1 WHERE name = $2 RETURNING id")
            .bind::<Text, _>("updated")
            .bind::<Text, _>("first")
            .execute(conn)
            .await?,
        1
    );
    assert_eq!(
        sql_query("DELETE FROM users WHERE name = $1")
            .bind::<Text, _>("missing")
            .execute(conn)
            .await?,
        0
    );
    let error = conn
        .transaction::<(), Error, _>(|conn| {
            async move {
                sql_query("INSERT INTO users (name) VALUES ($1)")
                    .bind::<Nullable<Text>, _>(None::<&str>)
                    .execute(conn)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::DatabaseError(DatabaseErrorKind::NotNullViolation, _)
    ));
    // The failed statement was rolled back and the connection remains usable.
    let count: Count = sql_query("SELECT count(*)::bigint AS count FROM users")
        .get_result(conn)
        .await?;
    assert_eq!(count.count, 2);
    Ok(())
}
