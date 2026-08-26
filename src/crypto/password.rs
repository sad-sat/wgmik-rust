pub fn verify_password(plain: &str, hashed: &str) -> bool {
    bcrypt::verify(plain, hashed).unwrap_or(false)
}

pub fn get_password_hash(plain: &str) -> Result<String, bcrypt::BcryptError> {
    bcrypt::hash(plain, 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let pwd = "SuperSecretPassword123!";
        let hash = get_password_hash(pwd).expect("hash");
        assert!(verify_password(pwd, &hash));
        assert!(!verify_password("WrongPassword123!", &hash));
    }
}
