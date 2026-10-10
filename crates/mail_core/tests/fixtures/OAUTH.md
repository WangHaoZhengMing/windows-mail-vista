# OAuth TLS fixture

`oauth-loopback.pfx` is a self-signed, synthetic loopback-only test identity. Its
password is `fixture-only`. It is intentionally untrusted and must never be
installed in an OS trust store or used outside the certificate-rejection test.

The certificate includes only `127.0.0.1` as a subject alternative name. The test
runs a local HTTPS listener and verifies that the production OAuth HTTP client
rejects its certificate before sending any authorization code or refresh token.

Generated using OpenSSL `req -x509 -newkey rsa:2048 -nodes` with
`-subj /CN=mail-test-loopback -addext subjectAltName=IP:127.0.0.1`, then
`pkcs12 -export -passout pass:fixture-only`. No real account credentials are used.
