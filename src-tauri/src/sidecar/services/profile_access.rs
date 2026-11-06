//! Independent bearer credentials for individual agent/profile MCP routes.
use crate::sidecar::db::profile_repo::ProfileRepository;
use crate::sidecar::db::Database;
use sha2::{Digest, Sha256};

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub fn rotate(db: &Database, profile_id: &str) -> Result<String, String> {
    if ProfileRepository::new(db).find_by_id(profile_id)?.is_none() {
        return Err("Profile not found".into());
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let digest = hash_token(&token);
    db.run(
        "INSERT INTO profile_access_tokens (profile_id, token_hash) VALUES (?1, ?2)
         ON CONFLICT(profile_id) DO UPDATE SET token_hash = excluded.token_hash",
        &[&profile_id, &digest],
    )?;
    Ok(token)
}

pub fn authorize(db: &Database, profile_id: &str, bearer: Option<&str>) -> bool {
    let Some(bearer) = bearer else {
        return false;
    };
    if bearer.len() != 64 || !bearer.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let expected = match db.query_one(
        "SELECT token_hash FROM profile_access_tokens WHERE profile_id = ?1",
        &[&profile_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(Some(hash)) => hash,
        _ => return false,
    };
    let actual = hash_token(bearer);
    // Fixed-length comparison to avoid disclosing how much of the digest matches.
    expected.len() == actual.len()
        && expected
            .bytes()
            .zip(actual.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::db::Database;
    #[test]
    fn isolated_tokens_and_rotation() {
        let db = Database::open(std::path::Path::new(":memory:")).unwrap();
        db.run_migrations().unwrap();
        let repo = ProfileRepository::new(&db);
        let a = repo.create("agent-a").unwrap();
        let b = repo.create("agent-b").unwrap();
        let first = rotate(&db, &a.id).unwrap();
        assert!(authorize(&db, &a.id, Some(&first)));
        assert!(!authorize(&db, &b.id, Some(&first)));
        assert!(!authorize(&db, &a.id, None));
        let second = rotate(&db, &a.id).unwrap();
        assert!(!authorize(&db, &a.id, Some(&first)));
        assert!(authorize(&db, &a.id, Some(&second)));
    }
}
