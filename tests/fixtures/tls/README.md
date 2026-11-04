# Local transport fixture

`server-key.der` is a deliberately public test-only PKCS#8 key, not a wallet key.
The certificate covers `api.example.com`, `spec.example.com`,
`grpc.example.com`, `*.example.com`, `localhost`, `127.0.0.1`, and `::1`.
Fake SOCKS maps fixture hostnames to local TLS listeners. The CA is valid for twenty years and the server certificate for 800 days from
their encoded creation dates (`openssl x509 -noout -dates`). The shorter leaf
lifetime satisfies macOS private-PKI certificate validation; renew it before
expiry rather than changing the verifier.
`untrusted.der` is an intentionally self-signed 800-day server leaf for
`api.example.com`, using the same public fixture key. It has CA:FALSE,
digitalSignature/keyEncipherment and serverAuth; it is absent from the fixture
trust roots. This isolates chain trust rejection from a mismatched key, hostname
or excessive leaf lifetime.
Library fixtures install this public CA in their default test context; integration
fixtures explicitly install immutable additional trust with `tls::install_trust`.
Independent contexts retain production trust defaults, including the untrusted
certificate tests. Normal chain and hostname verification remains enabled.
No user trust store is modified and no real fixture hostname is contacted.

To renew, generate a private fixture CA with OpenSSL, sign a server certificate
with an 800-day lifetime, the listed SANs, CA:FALSE,
digitalSignature/keyEncipherment, serverAuth, and subject/authority key identifiers,
then export the server certificate as DER and its key as unencrypted PKCS#8 DER.
Also regenerate the self-signed `untrusted.der` leaf with that server key and the
constraints above whenever the key is changed or either leaf expires. Commit the
CA certificate, both server certificates and public fixture server key; CA
signing keys are unnecessary for running the tests. Do not add `untrusted.der` to
the trusted CA bundle.
