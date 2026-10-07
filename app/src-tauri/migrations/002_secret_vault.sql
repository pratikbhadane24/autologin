-- Account secrets, encrypted with a single random key kept in the OS keychain
-- (see store/vault.rs). One keychain item instead of one per account means one
-- permission prompt per app version, however many accounts there are.
CREATE TABLE secrets (
    entry      TEXT PRIMARY KEY,   -- "<tenant>:<broker>:<client_id>"
    nonce      BLOB NOT NULL,      -- 24 bytes, random per write
    ciphertext BLOB NOT NULL       -- XChaCha20-Poly1305 of the JSON secrets map
);
