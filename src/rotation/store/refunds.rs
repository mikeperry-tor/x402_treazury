//! Historical read-only refund projection; current stores have no refund credit ledger.
use super::*;
#[derive(Clone, Debug, Serialize)]
pub struct RefundStatus {
    pub txid: String,
    pub output_index: u32,
    pub operation_id: String,
    pub amount: i64,
    pub height: i64,
}
pub(super) fn read_refunds(db: &Connection) -> Result<Vec<RefundStatus>> {
    Ok(db.prepare("SELECT txid,output_index,operation_id,amount,height FROM refund_outputs ORDER BY txid,output_index")?
        .query_map([], |r| Ok(RefundStatus { txid:r.get(0)?, output_index:r.get(1)?, operation_id:r.get(2)?, amount:r.get(3)?, height:r.get(4)? }))?
        .collect::<rusqlite::Result<_>>()?)
}
