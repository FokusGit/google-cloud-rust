use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime, Utc};
use google_cloud_googleapis::spanner::v1::commit_request::Transaction::SingleUseTransaction;
use google_cloud_googleapis::spanner::v1::Mutation;
use google_cloud_spanner::key::{Key, KeySet};
use google_cloud_spanner::mutation::insert_or_update;
use google_cloud_spanner::row::Row;
use google_cloud_spanner::statement::{Statement, ToKind};
use google_cloud_spanner::transaction::{CallOptions, QueryOptions};
use google_cloud_spanner::transaction_ro::{BatchReadOnlyTransaction, ReadOnlyTransaction};
use google_cloud_spanner::value::{CommitTimestamp, TimestampBound};
use rust_decimal::Decimal;
use serial_test::serial;
use std::ops::DerefMut;
use std::str::FromStr;

mod common;
use common::*;

fn create_user_item_mutation(user_id: &str, item_id: i64) -> Mutation {
    insert_or_update(
        "UserItem",
        vec!["UserId", "ItemId", "Quantity", "UpdatedAt"],
        vec![
            user_id.to_kind(),
            item_id.to_kind(),
            100.to_kind(),
            CommitTimestamp::new().to_kind(),
        ],
    )
}

fn create_user_character_mutation(user_id: &str, character_id: i64) -> Mutation {
    insert_or_update(
        "UserCharacter",
        vec!["UserId", "CharacterId", "Level", "UpdatedAt"],
        vec![
            user_id.to_kind(),
            character_id.to_kind(),
            1.to_kind(),
            CommitTimestamp::new().to_kind(),
        ],
    )
}

async fn assert_read(tx: &mut ReadOnlyTransaction, user_id: &str, now: &NaiveDateTime) {
    let reader = match tx
        .read(
            "User",
            user_columns(),
            KeySet::from(Key::one(user_id)),
            None,
        )
        .await
    {
        Ok(tx) => tx,
        Err(status) => panic!("read error {:?}", status),
    };
    let mut rows = all_rows(reader).await;
    assert_eq!(1, rows.len(), "row must exists");
    let row = rows.pop().unwrap();
    match get_row(&row, user_id, now) {
        Err(err) => panic!("row error {:?}", err),
        _ => {}
    }
}

async fn assert_query(tx: &mut ReadOnlyTransaction, user_id: &str, now: &NaiveDateTime) {
    let mut stmt = Statement::new("SELECT * FROM User WHERE UserId = @UserID");
    stmt.add_param("UserId", user_id);
    let mut rows = execute_query(tx, stmt).await;
    assert_eq!(1, rows.len(), "row must exists");
    let row = rows.pop().unwrap();
    match get_row(&row, user_id, now) {
        Err(err) => panic!("row error {:?}", err),
        _ => {}
    }
}

async fn execute_query(tx: &mut ReadOnlyTransaction, stmt: Statement) -> Vec<Row> {
    let reader = match tx.query(stmt, Some(QueryOptions::default())).await {
        Ok(tx) => tx,
        Err(status) => panic!("query error {:?}", status),
    };
    all_rows(reader).await
}

async fn assert_partitioned_query(
    tx: &mut BatchReadOnlyTransaction,
    user_id: &str,
    now: &NaiveDateTime,
) {
    let mut stmt = Statement::new("SELECT * FROM User WHERE UserId = @UserID");
    stmt.add_param("UserId", user_id);
    let row = execute_partitioned_query(tx, stmt).await;
    assert_eq!(row.len(), 1);
    match get_row(row.first().unwrap(), user_id, now) {
        Err(err) => panic!("row error {:?}", err),
        _ => {}
    }
}

async fn execute_partitioned_query(tx: &mut BatchReadOnlyTransaction, stmt: Statement) -> Vec<Row> {
    let partitions = match tx.partition_query(stmt, None, None).await {
        Ok(tx) => tx,
        Err(status) => panic!("query error {:?}", status),
    };
    println!("partition count = {}", partitions.len());
    let mut rows = vec![];
    for p in partitions.into_iter() {
        let reader = match tx.execute(p).await {
            Ok(tx) => tx,
            Err(status) => panic!("query error {:?}", status),
        };
        let rows_per_partition = all_rows(reader).await;
        for x in rows_per_partition {
            rows.push(x);
        }
    }
    rows
}

async fn assert_partitioned_read(
    tx: &mut BatchReadOnlyTransaction,
    user_id: &str,
    now: &NaiveDateTime,
) {
    let partitions = match tx
        .partition_read(
            "User",
            user_columns(),
            KeySet::from(Key::one(user_id)),
            None,
            None,
        )
        .await
    {
        Ok(tx) => tx,
        Err(status) => panic!("query error {:?}", status),
    };
    println!("partition count = {}", partitions.len());
    let mut rows = vec![];
    for p in partitions.into_iter() {
        let reader = match tx.execute(p).await {
            Ok(tx) => tx,
            Err(status) => panic!("query error {:?}", status),
        };
        let rows_per_partition = all_rows(reader).await;
        for x in rows_per_partition {
            rows.push(x);
        }
    }
    assert_eq!(rows.len(), 1);
    match get_row(rows.first().unwrap(), user_id, now) {
        Err(err) => panic!("row error {:?}", err),
        _ => {}
    }
}

fn get_row(row: &Row, source_user_id: &str, now: &NaiveDateTime) -> Result<(), anyhow::Error> {
    let user_id = row.column_by_name::<String>("UserId")?;
    assert_eq!(user_id.to_string(), source_user_id);
    let not_null_int64 = row.column_by_name::<i64>("NotNullINT64")?;
    assert_eq!(not_null_int64, 1);
    let nullable_int64 = row.column_by_name::<Option<i64>>("NullableINT64")?;
    assert_eq!(nullable_int64, None);
    let not_null_float64 = row.column_by_name::<f64>("NotNullFloat64")?;
    assert_eq!(not_null_float64, 1.0);
    let nullable_float64 = row.column_by_name::<Option<f64>>("NullableFloat64")?;
    assert_eq!(nullable_float64, None);
    let not_null_bool = row.column_by_name::<bool>("NotNullBool")?;
    assert_eq!(not_null_bool, true);
    let nullable_bool = row.column_by_name::<Option<bool>>("NullableBool")?;
    assert_eq!(nullable_bool, None);
    let mut not_null_byte_array = row.column_by_name::<Vec<u8>>("NotNullByteArray")?;
    assert_eq!(not_null_byte_array.pop().unwrap(), 1 as u8);
    let nullable_byte_array = row.column_by_name::<Option<Vec<u8>>>("NullableByteArray")?;
    assert_eq!(nullable_byte_array, None);
    let not_null_decimal = row.column_by_name::<Decimal>("NotNullNumeric")?;
    assert_eq!(not_null_decimal.to_string(), "100.24");
    let nullable_decimal = row.column_by_name::<Option<Decimal>>("NullableNumeric")?;
    assert_eq!(nullable_decimal.unwrap().to_string(), "1000.42342");
    let not_null_ts = row.column_by_name::<NaiveDateTime>("NotNullTimestamp")?;
    assert_eq!(not_null_ts.to_string(), now.to_string());
    let nullable_ts = row.column_by_name::<Option<NaiveDateTime>>("NullableTimestamp")?;
    assert_eq!(nullable_ts.unwrap().to_string(), now.to_string());
    let not_null_date = row.column_by_name::<NaiveDate>("NotNullDate")?;
    assert_eq!(not_null_date.to_string(), now.date().to_string());
    let nullable_date = row.column_by_name::<Option<NaiveDate>>("NullableDate")?;
    assert_eq!(nullable_date, None);
    let mut not_null_array = row.column_by_name::<Vec<i64>>("NotNullArray")?;
    assert_eq!(not_null_array.pop().unwrap(), 30); // from tail
    assert_eq!(not_null_array.pop().unwrap(), 20);
    assert_eq!(not_null_array.pop().unwrap(), 10);
    let nullable_array = row.column_by_name::<Option<Vec<i64>>>("NullableArray")?;
    assert_eq!(nullable_array, None);
    let nullable_string = row.column_by_name::<Option<String>>("NullableString")?;
    assert_eq!(nullable_string.unwrap(), user_id);
    let updated_at = row.column_by_name::<CommitTimestamp>("UpdatedAt")?;
    assert_ne!(updated_at.timestamp.to_string(), now.to_string());
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_query_and_read() {
    let now = Utc::now().naive_utc();
    let mut session = create_session().await;
    let user_id_1 = "user_1";
    let user_id_2 = "user_2";
    let user_id_3 = "user_3";
    replace_test_data(
        session.deref_mut(),
        vec![
            create_user_mutation(&user_id_1, &now),
            create_user_mutation(&user_id_2, &now),
            create_user_mutation(&user_id_3, &now),
        ],
    )
    .await
    .unwrap();

    let mut tx = match ReadOnlyTransaction::begin(
        session,
        TimestampBound::strong_read(),
        CallOptions::default(),
    )
    .await
    {
        Ok(tx) => tx,
        Err(status) => panic!("begin error {:?}", status),
    };

    assert_query(&mut tx, user_id_1, &now).await;
    assert_query(&mut tx, user_id_2, &now).await;
    assert_query(&mut tx, user_id_3, &now).await;
    assert_read(&mut tx, user_id_1, &now).await;
    assert_read(&mut tx, user_id_2, &now).await;
    assert_read(&mut tx, user_id_3, &now).await;
}

#[tokio::test]
#[serial]
async fn test_complex_query() {
    let now = Utc::now().naive_utc();
    let mut session = create_session().await;
    let user_id_1 = "user_10";
    replace_test_data(
        session.deref_mut(),
        vec![
            create_user_mutation(&user_id_1, &now),
            create_user_item_mutation(&user_id_1, 1),
            create_user_item_mutation(&user_id_1, 2),
            create_user_character_mutation(&user_id_1, 10),
            create_user_character_mutation(&user_id_1, 20),
        ],
    )
    .await
    .unwrap();

    let mut tx = match ReadOnlyTransaction::begin(
        session,
        TimestampBound::strong_read(),
        CallOptions::default(),
    )
    .await
    {
        Ok(tx) => tx,
        Err(status) => panic!("begin error {:?}", status),
    };

    let mut stmt = Statement::new(
        "SELECT *,
        ARRAY(SELECT AS STRUCT * FROM UserItem WHERE UserId = p.UserId) as UserItem,
        ARRAY(SELECT AS STRUCT * FROM UserCharacter WHERE UserId = p.UserId) as UserCharacter,
        FROM User p WHERE UserId = @UserId;
    ",
    );
    stmt.add_param("UserId", user_id_1);
    let mut rows = execute_query(&mut tx, stmt).await;
    assert_eq!(1, rows.len());
    let row = rows.pop().unwrap();

    // check UserTable
    match get_row(&row, user_id_1, &now) {
        Err(err) => panic!("row error {:?}", err),
        _ => {}
    }

    let mut user_items = row.column_by_name::<Vec<UserItem>>("UserItem").unwrap();
    let first_item = user_items.pop().unwrap();
    assert_eq!(first_item.user_id, user_id_1);
    assert_eq!(first_item.item_id, 2);
    assert_eq!(first_item.quantity, 100);
    assert_ne!(first_item.updated_at.timestamp.to_string(), now.to_string());
    let second_item = user_items.pop().unwrap();
    assert_eq!(second_item.user_id, user_id_1);
    assert_eq!(second_item.item_id, 1);
    assert_eq!(second_item.quantity, 100);
    assert_ne!(
        second_item.updated_at.timestamp.to_string(),
        now.to_string()
    );
    assert!(user_items.is_empty());

    let mut user_characters = row
        .column_by_name::<Vec<UserCharacter>>("UserCharacter")
        .unwrap();
    let first_character = user_characters.pop().unwrap();
    assert_eq!(first_character.user_id, user_id_1);
    assert_eq!(first_character.character_id, 20);
    assert_eq!(first_character.level, 1);
    assert_ne!(
        first_character.updated_at.timestamp.to_string(),
        now.to_string()
    );
    let second_character = user_characters.pop().unwrap();
    assert_eq!(second_character.user_id, user_id_1);
    assert_eq!(second_character.character_id, 10);
    assert_eq!(second_character.level, 1);
    assert_ne!(
        second_character.updated_at.timestamp.to_string(),
        now.to_string()
    );
    assert!(user_characters.is_empty());
}

#[tokio::test]
#[serial]
async fn test_batch_partition_query_and_read() {
    let now = Utc::now().naive_utc();
    let mut session = create_session().await;
    let user_id_1 = "user_1";
    let user_id_2 = "user_2";
    let user_id_3 = "user_3";
    replace_test_data(
        session.deref_mut(),
        vec![
            create_user_mutation(&user_id_1, &now),
            create_user_mutation(&user_id_2, &now),
            create_user_mutation(&user_id_3, &now),
        ],
    )
    .await
    .unwrap();

    let mut tx = match BatchReadOnlyTransaction::begin(
        session,
        TimestampBound::strong_read(),
        CallOptions::default(),
    )
    .await
    {
        Ok(tx) => tx,
        Err(status) => panic!("begin error {:?}", status),
    };

    assert_partitioned_query(&mut tx, user_id_1, &now).await;
    assert_partitioned_query(&mut tx, user_id_2, &now).await;
    assert_partitioned_query(&mut tx, user_id_3, &now).await;
    assert_partitioned_read(&mut tx, user_id_1, &now).await;
    assert_partitioned_read(&mut tx, user_id_2, &now).await;
    assert_partitioned_read(&mut tx, user_id_3, &now).await;
}
