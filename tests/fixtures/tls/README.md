# Local transport fixture

`server-key.der` is a deliberately public test-only PKCS#8 key, not a wallet key.
The certificate covers `api.example.com`, `spec.example.com`, and
`grpc.example.com`; fake SOCKS maps those names to a local TLS listener. It is
valid for twenty years from 2026-10-03. The test CA is accepted only by cfg(test)
network contexts, using a dedicated root store with normal certificate/hostname
verification. These fixture success cases do not qualify an OS trust store.
Untrusted-certificate cases also exercise the unchanged production trust defaults.
Production trust configuration is unchanged. No real hostname
is contacted and no hostname/certificate checks are disabled.

To renew, generate a private fixture CA with OpenSSL, sign a server certificate
with the three SANs, CA:FALSE, digitalSignature/keyEncipherment and serverAuth,
then export the server certificate as DER and its key as unencrypted PKCS#8 DER.
Commit only the CA certificate, server certificate and public fixture server key;
CA signing keys are unnecessary for running the tests.
