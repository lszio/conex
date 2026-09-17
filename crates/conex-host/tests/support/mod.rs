//! Shared OIDC fixture: a throwaway RSA-2048 key (never a real secret) used
//! by tests/oidc_jwt.rs and tests/host_oidc.rs to sign/verify RS256
//! id_tokens. Generated once with `openssl genpkey -algorithm RSA -pkeyopt
//! rsa_keygen_bits:2048`.

use base64::Engine as _;
use conex_host::oidc_jwt::sign_rs256;
use ring::rand::SystemRandom;
use std::sync::LazyLock;

/// PKCS#8 DER of the fixture RSA-2048 key, base64.
pub const TEST_PKCS8_B64: &str = "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCn55fTVo++ujoDONMcuE1h426epreFCCxSR/R1ES+D64zn9YI7pliHM4u0VGozu3CvSqzkebfhKuxvUkrXqHvli8cTKt8lHBNnG1fK2f3kdgm3L9dxsDuoVkwJLga+wC60Mbk3sZmOWGW6lWPJq6LX85CDgZEX16hBT0SCslLmIIqLJxV9Qe+BIqrzMV/gTnO1ChBma3M9SEwQNIJ5wwHXIsi/9XA5jsyKl2++FSMoSIo6/2sZsIgv6USTFYRRoaHCx9zoMUrwGnUqHGkCo+fLY834l1G9gmal05qGbEMGtLbVnSjFO4CXIejmGysS4dmOkUNk5qrET00EFbGSyIuhAgMBAAECggEACxvXf7QKbqQDkpL/5LaM6B0UBIsjOToBNLBuDBDHBANhnzcvFqM62Dcg1x1+qU2NdwuJxNQIxm0Al5zqNrp++AD3et0rRllFL10vwcwKcEMfAW+44Vy3lbuf7DuPVq+AZ4uipXyqnDBdCqs3nvAsJf2HYiwAXYN3LlsAPWVu9qq04KkkcbeofGCWDnWYWCgtzJ3NOvjzUfnUcYhxnYH5MlRYTJ+wk98rk+riAzTcxHG14alSroHXLDXYrnu3TL4Ns0kLP4xgibYopGD8DzOA9HvFHKX7XtYRzM4yGtVoZjE9i9VP3Wy1nSaPYSfTdsmZG8UHgTVyx/v1DK47y3G1AQKBgQDpC/QVGEY25GLeOI6dyYT5GBPNvAT7sPjDILvuLS8kPjvvv0X3I2XWgZvpc21jFADSqwIiT4bBzd5aB26ec8Xmb9dUuBNbt4TAVcPUbeD6lqc071vgAR1pSQMyUOt51X8yX4GoSEqPUO37l+spINCO0jzXWCz/YGpmms/7K1s57QKBgQC4cSV5nPKd7IJA+i8b9qNXYMOc1X09R4UAPCvgdt/8hEO9jPdufDJwqD8wcPrhiYGs+OR3j1Py5YZgXv5orGulhJuZoQZQKRM0rf3eQLajDUE6EvfmloYDziwaNvLlyp4nHSRB+sgkpdacjf+Ox7tVwSm4MrDunKXInfqVtuzSBQKBgC6ND7WDAsuGNWWUQJCuJ1ymfZYz/37TK+22RTPfXLJNqCVMvMoQDRCbFSy9vNT0svFh7WwzHITr/YVYRLVsBNTx9D5dAqjocKEGwLZXOIB1xXKieWS2dEyKpBPR7CeLCPxj7X9S6WnVTaRbBUNS5bYRssuFNn/Qn5BdTjwqve9FAoGBAKuwt5/DV51mYcG2ok+3gUl/S9gca161yrrzSCzEu7BGNwClzlZMym9QTrH7Ga8E329yqMoa45yByFrBUrWBexsym92gpU3NTpGFPYK8XsbdOdCjg5xklg/IxgkJCYaa3Cmw2OWKWvCyZ1qIXFI+3sXu77UFiuoza6eaV38yLrU5AoGBAMA36TBrdxGGFfEqPHswd8WltyH3m+/rGD4xU4TXbUYv+i/d9x3hOZ0mkRyezuu/lMVmYv61aH6kFD8HmMPQf2YvhmioSXe3Q4Ul1SYjoVRqPgR1Yg7zMHlLTSP6m42RGDW1PVF0aJfPLeoTc/yQbrGYkuKpx/xypKTnf7CLr3WM";

pub const TEST_KID: &str = "UlERqg4IKXazWWTQCkUD7A";

/// The JWKS document matching the fixture key.
pub const TEST_JWKS_JSON: &str = r#"{"keys":[{"kty":"RSA","alg":"RS256","kid":"UlERqg4IKXazWWTQCkUD7A","use":"sig","n":"AKfnl9NWj766OgM40xy4TWHjbp6mt4UILFJH9HURL4PrjOf1gjumWIczi7RUajO7cK9KrOR5t-Eq7G9SSteoe-WLxxMq3yUcE2cbV8rZ_eR2Cbcv13GwO6hWTAkuBr7ALrQxuTexmY5YZbqVY8mrotfzkIOBkRfXqEFPRIKyUuYgiosnFX1B74EiqvMxX-BOc7UKEGZrcz1ITBA0gnnDAdciyL_1cDmOzIqXb74VIyhIijr_axmwiC_pRJMVhFGhocLH3OgxSvAadSocaQKj58tjzfiXUb2CZqXTmoZsQwa0ttWdKMU7gJch6OYbKxLh2Y6RQ2TmqsRPTQQVsZLIi6E","e":"AQAB"}]}"#;

static RNG: LazyLock<SystemRandom> = LazyLock::new(SystemRandom::new);

/// Sign an RS256 JWT for the fixture key with the given `kid` header.
pub fn sign_id_token(kid: &str, payload: &str, alg: &str) -> String {
    sign_rs256(&pkcs8(), kid, alg, payload, &RNG).expect("sign id_token")
}

fn pkcs8() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(TEST_PKCS8_B64)
        .expect("decode fixture key")
}
