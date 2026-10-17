//! Bounded reads of the original return journals. The accumulator lives outside
//! the serialized Store worker; every page belongs to one exact inventory.
use super::*;

#[derive(Clone)]
enum Cursor {
    Confirmed(String),
    Current(i64),
    Archived(i64),
    Credits(i64),
    Done,
}
enum Item {
    Confirmed(String),
    Binding(String, Option<String>),
    Credit(RefundStatus, i64),
}
struct Page {
    next: Cursor,
    items: Vec<Item>,
}

impl Store {
    pub(crate) fn return_inventory_revision(&self) -> Result<i64> {
        Ok(self.db.query_row(
            "SELECT inventory_revision FROM source_support_current WHERE singleton=1",
            [],
            |row| row.get(0),
        )?)
    }
    pub(crate) fn check_return_inventory(&self, revision: i64) -> Result<()> {
        ensure!(
            self.return_inventory_revision()? == revision,
            "return inventory changed"
        );
        Ok(())
    }
    fn return_inventory_page(&self, revision: i64, cursor: Cursor) -> Result<Page> {
        self.check_return_inventory(revision)?;
        let mut items = Vec::new();
        let next = match cursor {
            Cursor::Confirmed(after) => {
                let rows = self.db.prepare("SELECT id FROM treasury_operations WHERE id>?1 AND submission='CONFIRMED' ORDER BY id LIMIT 64")?
                    .query_map([after], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let next = rows
                    .last()
                    .cloned()
                    .map(Cursor::Confirmed)
                    .unwrap_or(Cursor::Current(0));
                items.extend(rows.into_iter().map(Item::Confirmed));
                next
            }
            Cursor::Current(after) | Cursor::Archived(after) => {
                let archived = matches!(cursor, Cursor::Archived(_));
                let sql = if archived {
                    "SELECT rowid,job_id,operation_id,refund FROM funding_recovery WHERE rowid>?1 ORDER BY rowid LIMIT 64"
                } else {
                    "SELECT j.rowid,j.job_id,j.operation_id,r.address FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.rowid>?1 ORDER BY j.rowid LIMIT 64"
                };
                let rows = self
                    .db
                    .prepare(sql)?
                    .query_map([after], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<Vec<u8>>>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let next = match rows.last() {
                    Some((id, _, _, _)) if archived => Cursor::Archived(*id),
                    Some((id, _, _, _)) => Cursor::Current(*id),
                    None if archived => Cursor::Credits(0),
                    None => Cursor::Archived(0),
                };
                for (_, job, operation, bytes) in rows {
                    let address = bytes
                        .map(|bytes| -> Result<String> {
                            let plain = unseal(
                                &self.key,
                                &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                                &bytes,
                            )?;
                            Ok(std::str::from_utf8(&plain)?.to_owned())
                        })
                        .transpose()?;
                    items.push(Item::Binding(operation, address));
                }
                next
            }
            Cursor::Credits(after) => {
                let rows=self.db.prepare("SELECT rowid,txid,output_index,operation_id,amount,height,credited FROM refund_outputs WHERE rowid>?1 ORDER BY rowid LIMIT 64")?
                    .query_map([after],|row| Ok((row.get::<_,i64>(0)?,RefundStatus {txid:row.get(1)?,output_index:row.get(2)?,operation_id:row.get(3)?,amount:row.get(4)?,height:row.get(5)?},row.get::<_,i64>(6)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let next = rows
                    .last()
                    .map(|row| Cursor::Credits(row.0))
                    .unwrap_or(Cursor::Done);
                items.extend(
                    rows.into_iter()
                        .map(|(_, row, credit)| Item::Credit(row, credit)),
                );
                next
            }
            Cursor::Done => Cursor::Done,
        };
        Ok(Page { next, items })
    }
}

impl ReturnAccounting {
    fn finish_inventory(&mut self, mut missing: std::collections::BTreeSet<String>) {
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for owners in self.bindings.values() {
            if owners.len() != 1 {
                self.ambiguous_operations.extend(owners.iter().cloned());
            }
            for operation in owners {
                *counts.entry(operation.clone()).or_default() += 1;
            }
        }
        self.ambiguous_operations.extend(
            counts
                .iter()
                .filter(|(_, count)| **count != 1)
                .map(|(operation, _)| operation.clone()),
        );
        missing.extend(
            self.credited
                .iter()
                .map(|credit| credit.operation_id.clone()),
        );
        self.issues.extend(
            missing
                .into_iter()
                .filter(|operation| !counts.contains_key(operation))
                .map(|operation_id| ReturnAccountingIssue::MissingOriginalBinding { operation_id }),
        );
        self.issues.extend(
            self.ambiguous_operations
                .iter()
                .cloned()
                .map(
                    |operation_id| ReturnAccountingIssue::AmbiguousOriginalBinding { operation_id },
                ),
        );
        self.issues.sort();
    }
}

pub(crate) async fn load(store: &StoreHandle) -> Result<(i64, ReturnAccounting)> {
    let revision = store
        .call(|store| store.return_inventory_revision())
        .await?;
    let mut cursor = Cursor::Confirmed(String::new());
    let mut accounting = ReturnAccounting::default();
    let mut missing = std::collections::BTreeSet::new();
    loop {
        let page = store
            .call(move |store| store.return_inventory_page(revision, cursor))
            .await?;
        for item in page.items {
            match item {
                Item::Confirmed(operation) => {
                    accounting.confirmed.insert(operation);
                }
                Item::Binding(operation, Some(address)) => {
                    accounting
                        .bindings
                        .entry(address)
                        .or_default()
                        .insert(operation);
                }
                Item::Binding(operation, None) if accounting.confirmed.contains(&operation) => {
                    missing.insert(operation);
                }
                Item::Binding(_, None) => {}
                Item::Credit(credit, amount) => {
                    let total = accounting
                        .credited_totals
                        .entry(credit.operation_id.clone())
                        .or_default();
                    *total = total
                        .checked_add(amount)
                        .context("return credit total overflow")?;
                    accounting.credited.push(credit);
                }
            }
        }
        cursor = page.next;
        if matches!(cursor, Cursor::Done) {
            break;
        }
        tokio::task::yield_now().await;
    }
    accounting.finish_inventory(missing);
    store
        .call(move |store| store.check_return_inventory(revision))
        .await?;
    Ok((revision, accounting))
}
