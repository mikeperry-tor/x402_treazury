//! Independent Python cryptography/OpenSSL vectors for the existing v1 layout.
//! Public key 00..1f; fixed nonces are fixtures only. No production key material.
use super::*;

const KEY: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const ID: &str = "00000000-0000-4000-8000-000000000001";
// nonce12 || ciphertext || tag16, with every existing record AAD family.
const RECORDS: &[(&str, &[u8], &[u8])] = &[
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:snapshot:1",
        b"legacy v1 snapshot",
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 229, 158, 111, 97, 74, 110, 133, 54, 134, 163,
            76, 157, 249, 109, 125, 11, 166, 4, 205, 240, 24, 174, 110, 36, 109, 91, 146, 113, 205,
            195, 59, 242, 37, 122,
        ],
    ),
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:refund:job",
        b"legacy v1 refund:job",
        &[
            12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 48, 54, 95, 247, 61, 125, 62, 91, 180,
            161, 195, 30, 201, 46, 159, 122, 135, 82, 93, 130, 16, 134, 88, 219, 62, 49, 4, 158,
            158, 197, 145, 191, 187, 15, 230, 239,
        ],
    ),
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:outgoing:operation",
        b"legacy v1 outgoing:operation",
        &[
            24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 210, 13, 137, 8, 118, 29, 215, 203,
            242, 237, 25, 201, 194, 217, 49, 162, 64, 8, 14, 92, 179, 180, 83, 54, 162, 1, 133, 88,
            34, 181, 254, 141, 164, 64, 126, 59, 217, 234, 45, 56, 98, 10, 145, 140,
        ],
    ),
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:pool:evm:wallet",
        b"legacy v1 pool:evm:wallet",
        &[
            36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 147, 188, 77, 219, 42, 9, 110, 154,
            202, 228, 51, 248, 149, 220, 84, 150, 181, 206, 85, 73, 128, 231, 44, 179, 32, 85, 15,
            8, 16, 113, 68, 210, 192, 168, 186, 2, 75, 3, 82, 196, 203,
        ],
    ),
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:funding:operation",
        b"legacy v1 funding:operation",
        &[
            48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 58, 145, 45, 43, 167, 152, 247, 147,
            89, 50, 223, 121, 27, 252, 65, 108, 172, 249, 191, 101, 168, 243, 112, 93, 245, 106,
            138, 254, 55, 228, 248, 58, 230, 240, 6, 196, 12, 206, 100, 57, 16, 120, 252,
        ],
    ),
    (
        "v1:00000000-0000-4000-8000-000000000001:mainnet:preparation:operation",
        b"legacy v1 preparation:operation",
        &[
            60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 71, 169, 207, 57, 40, 76, 42, 6, 173,
            248, 173, 93, 222, 27, 132, 27, 205, 112, 249, 125, 202, 136, 77, 185, 149, 225, 68,
            99, 23, 139, 250, 201, 224, 41, 200, 109, 76, 66, 48, 186, 50, 66, 241, 165, 10, 248,
            238,
        ],
    ),
];

#[test]
fn v1_independent_ciphertexts_preserve_record_bindings_and_reject_damage() {
    for &(aad, plaintext, record) in RECORDS {
        assert_eq!(unseal(&KEY, aad, record).unwrap().as_slice(), plaintext);
        let mut wrong_key = KEY;
        wrong_key[0] ^= 1;
        assert!(unseal(&wrong_key, aad, record).is_err());
        for changed_aad in [
            aad.replacen("v1:", "v2:", 1),
            aad.replacen(ID, "another-treasury", 1),
            aad.replacen(":mainnet:", ":testnet:", 1),
            format!("{aad}-another-record"),
        ] {
            assert!(unseal(&KEY, &changed_aad, record).is_err());
        }
        // Damage nonce, ciphertext, and final authentication tag independently.
        for offset in [0, 12, record.len() - 1] {
            let mut damaged = record.to_vec();
            damaged[offset] ^= 1;
            assert!(unseal(&KEY, aad, &damaged).is_err());
        }
        for length in 0..record.len() {
            assert!(unseal(&KEY, aad, &record[..length]).is_err());
        }
        // New writes retain the legacy prefix/tag boundaries and authentication.
        let written = seal(&KEY, aad, plaintext).unwrap();
        assert_eq!(written.len(), 12 + plaintext.len() + 16);
        assert_eq!(unseal(&KEY, aad, &written).unwrap().as_slice(), plaintext);
    }
}

#[test]
fn independent_v1_snapshot_reopens_and_survives_backup_and_new_write() {
    use std::io::Write;
    let temporary = tempfile::tempdir().unwrap();
    let dir = temporary.path().join("state");
    let key_file = temporary.path().join("key");
    crate::private_fs::create_dir(&dir).unwrap();
    private_file(&dir.join("owner.lock"), true).unwrap();
    private_file(&key_file, true)
        .unwrap()
        .write_all(&KEY)
        .unwrap();
    private_file(&dir.join("state.sqlite"), true).unwrap();
    {
        let db = Connection::open(dir.join("state.sqlite")).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.execute("INSERT INTO instance VALUES (?1,1,1,'mainnet',0)", [ID])
            .unwrap();
        db.execute("INSERT INTO snapshots VALUES (1,?1)", [RECORDS[0].2])
            .unwrap();
    }
    let mut store = Store::open(&dir, &key_file, ID).unwrap();
    assert_eq!(store.snapshot().unwrap().1.as_slice(), RECORDS[0].1);
    for (pragma, expected) in [
        ("foreign_keys", 1),
        ("synchronous", 2),
        ("busy_timeout", 5000),
    ] {
        assert_eq!(
            store
                .db
                .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get::<_, i64>(0))
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        store
            .db
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    let backup = temporary.path().join("backup");
    store.backup(&backup).unwrap();
    let restored = Store::open(&backup, &backup.join("key"), ID).unwrap();
    assert_eq!(restored.snapshot().unwrap().1.as_slice(), RECORDS[0].1);
    drop(restored);
    store.save_snapshot(1, b"new snapshot").unwrap();
    drop(store);
    let reopened = Store::open(&dir, &key_file, ID).unwrap();
    assert_eq!(reopened.snapshot().unwrap().0, 2);
    assert_eq!(reopened.snapshot().unwrap().1.as_slice(), b"new snapshot");
}
