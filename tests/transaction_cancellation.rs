use super::TestConnection;
use diesel::query_builder::{AsQuery, QueryFragment, QueryId};
use diesel::result::ConnectionResult;
use diesel::QueryResult;
use diesel_async::{
    AnsiTransactionManager, AsyncConnection, SimpleAsyncConnection, TransactionManager,
};
use std::future::{pending, Future};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

// Pause after the server executes a transaction command, before the transaction
// manager receives its result and updates its local transaction depth.
struct PauseAfterQuery {
    inner: TestConnection,
    executed: Arc<Notify>,
}

#[async_trait::async_trait]
impl SimpleAsyncConnection for PauseAfterQuery {
    async fn batch_execute(&mut self, query: &str) -> QueryResult<()> {
        self.inner.batch_execute(query).await?;
        self.executed.notify_one();
        pending().await
    }
}

#[async_trait::async_trait]
impl AsyncConnection for PauseAfterQuery {
    type ExecuteFuture<'conn, 'query> =
        <TestConnection as AsyncConnection>::ExecuteFuture<'conn, 'query>;
    type LoadFuture<'conn, 'query> = <TestConnection as AsyncConnection>::LoadFuture<'conn, 'query>;
    type Stream<'conn, 'query> = <TestConnection as AsyncConnection>::Stream<'conn, 'query>;
    type Row<'conn, 'query> = <TestConnection as AsyncConnection>::Row<'conn, 'query>;
    type Backend = <TestConnection as AsyncConnection>::Backend;
    type TransactionManager = AnsiTransactionManager;

    async fn establish(database_url: &str) -> ConnectionResult<Self> {
        Ok(Self {
            inner: TestConnection::establish(database_url).await?,
            executed: Arc::new(Notify::new()),
        })
    }

    fn load<'conn, 'query, T>(&'conn mut self, source: T) -> Self::LoadFuture<'conn, 'query>
    where
        T: AsQuery + 'query,
        T::Query: QueryFragment<Self::Backend> + QueryId + 'query,
    {
        self.inner.load(source)
    }

    fn execute_returning_count<'conn, 'query, T>(
        &'conn mut self,
        source: T,
    ) -> Self::ExecuteFuture<'conn, 'query>
    where
        T: QueryFragment<Self::Backend> + QueryId + 'query,
    {
        self.inner.execute_returning_count(source)
    }

    fn transaction_state(&mut self) -> &mut AnsiTransactionManager {
        self.inner.transaction_state()
    }
}

async fn cancel_after_execution(
    future: impl Future<Output = QueryResult<()>>,
    executed: Arc<Notify>,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            result = future => panic!("expected an in-flight transaction command, got {result:?}"),
            _ = executed.notified() => {}
        }
    })
    .await
    .expect("transaction command did not reach the server");
}

async fn connection() -> PauseAfterQuery {
    PauseAfterQuery::establish(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn cancelled_begin_marks_connection_broken() {
    let mut conn = connection().await;
    let executed = conn.executed.clone();
    cancel_after_execution(
        AnsiTransactionManager::begin_transaction(&mut conn),
        executed,
    )
    .await;
    assert!(AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}

#[tokio::test]
async fn cancelled_custom_begin_marks_connection_broken() {
    let mut conn = connection().await;
    let executed = conn.executed.clone();
    cancel_after_execution(
        AnsiTransactionManager::begin_transaction_sql(&mut conn, "BEGIN READ ONLY"),
        executed,
    )
    .await;
    assert!(AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}

#[tokio::test]
async fn cancelled_savepoint_breaks_even_a_test_transaction() {
    let mut conn = connection().await;
    conn.inner.begin_test_transaction().await.unwrap();
    assert!(!AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
    let executed = conn.executed.clone();
    cancel_after_execution(
        AnsiTransactionManager::begin_transaction(&mut conn),
        executed,
    )
    .await;
    assert!(AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}

#[tokio::test]
async fn cancelled_commit_marks_connection_broken() {
    let mut conn = connection().await;
    conn.inner.begin_test_transaction().await.unwrap();
    let executed = conn.executed.clone();
    cancel_after_execution(
        AnsiTransactionManager::commit_transaction(&mut conn),
        executed,
    )
    .await;
    assert!(AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}

#[tokio::test]
async fn cancelled_rollback_marks_connection_broken() {
    let mut conn = connection().await;
    conn.inner.begin_test_transaction().await.unwrap();
    let executed = conn.executed.clone();
    cancel_after_execution(
        AnsiTransactionManager::rollback_transaction(&mut conn),
        executed,
    )
    .await;
    assert!(AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}

#[tokio::test]
async fn failed_begin_does_not_poison_a_usable_connection() {
    let mut conn = TestConnection::establish(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    assert!(
        AnsiTransactionManager::begin_transaction_sql(&mut conn, "INVALID BEGIN")
            .await
            .is_err()
    );
    assert!(!AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
    AnsiTransactionManager::begin_transaction_sql(&mut conn, "BEGIN READ ONLY")
        .await
        .unwrap();
    AnsiTransactionManager::commit_transaction(&mut conn)
        .await
        .unwrap();
    assert!(!AnsiTransactionManager::is_broken_transaction_manager(
        &mut conn
    ));
}
