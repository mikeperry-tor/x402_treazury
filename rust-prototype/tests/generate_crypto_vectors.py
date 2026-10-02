"""Regenerate independent test vectors with the project's Python dependencies.
Run from repo root: .venv/bin/python rust-prototype/tests/generate_crypto_vectors.py
The private key here is public test data, never a funded wallet.
"""
import json
from pathlib import Path
from Crypto.Hash import keccak
from eth_account import Account
from eth_account.messages import encode_typed_data

def hash_bytes(data):
    return '0x' + keccak.new(digest_bits=256, data=data).hexdigest()

message = {
    'types': {
        'EIP712Domain': [{'name': n, 'type': t} for n, t in [
            ('name', 'string'), ('version', 'string'), ('chainId', 'uint256'), ('verifyingContract', 'address')]],
        'TransferWithAuthorization': [{'name': n, 'type': t} for n, t in [
            ('from', 'address'), ('to', 'address'), ('value', 'uint256'),
            ('validAfter', 'uint256'), ('validBefore', 'uint256'), ('nonce', 'bytes32')]],
    },
    'primaryType': 'TransferWithAuthorization',
    'domain': {'name': 'USD Coin', 'version': '2', 'chainId': 8453,
               'verifyingContract': '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913'},
    'message': {'from': Account.from_key(bytes.fromhex('00' * 31 + '01')).address,
                'to': '0x0000000000000000000000000000000000000003', 'value': 14000,
                'validAfter': 1700000000, 'validBefore': 1700000600, 'nonce': '0x' + '42' * 32},
}
encoded = encode_typed_data(full_message=message)
signed = Account.sign_message(encoded, bytes.fromhex('00' * 31 + '01'))
vectors = {
    'generator': 'PyCryptodome keccak + eth-account encode_typed_data/sign_message',
    'keccak': [{'length': n, 'hash': hash_bytes(bytes(i % 256 for i in range(n)))}
               for n in [0, 3, 135, 136, 137, 272, 4096]],
    'eip3009': {'typed_data': message, 'hash': '0x' + signed.message_hash.hex(),
                'signature': '0x' + signed.signature.hex()},
}
Path(__file__).with_name('fixtures').joinpath('crypto_vectors.json').write_text(json.dumps(vectors, indent=2) + '\n')
