# Local transport fixture

`server-key.der` is a public test-only PKCS#8 key, never a wallet key. The
fixture server certificate covers `api.example.com`, `spec.example.com`,
`grpc.example.com`, `*.example.com`, `localhost`, `127.0.0.1`, and `::1`, and
fake SOCKS maps fixture hostnames to local TLS listeners.

- **Lifetimes:** The fixture CA is valid for 20 years and the server leaf for
  800 days from their creation dates (`openssl x509 -noout -dates`). The
  800-day leaf lifetime satisfies macOS private-PKI validation; renew before
  expiry rather than weakening the verifier.
- **Untrusted leaf (`untrusted.der`):** A self-signed 800-day leaf for
  `api.example.com` using the same fixture key (`CA:FALSE`, `digitalSignature`,
  `keyEncipherment`, `serverAuth`), excluded from the fixture trust roots so
  trust-rejection tests isolate chain trust from key, hostname, or lifetime
  errors.
- **Trust scoping:** Library fixtures install the fixture CA in their default
  test context; integration fixtures opt in with `tls::install_trust`.
  Independent contexts keep production trust defaults, and no host trust store
  is modified or external host contacted.

To renew, generate a private fixture CA with OpenSSL, sign an 800-day server
leaf with the listed SANs, `CA:FALSE`, `digitalSignature`, `keyEncipherment`,
`serverAuth`, and subject and authority key identifiers, and export the server
certificate as DER and its key as unencrypted PKCS#8 DER. Regenerate
`untrusted.der` with the same server key and constraints whenever the key
changes or either leaf expires. Commit the CA certificate, both server leaves,
and the public fixture key (CA signing keys are not needed); never add
`untrusted.der` to the trusted CA bundle.
