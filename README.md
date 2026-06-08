# arkzap-me

`arkzap-me` is a small LNURL-pay server for Bark and Arkade. It gives every
supported Ark address a deterministic Lightning address, serves LNURL-pay
metadata, and generates BOLT11 invoices for incoming payments.

For Bark addresses, the service asks `barkd` to generate invoices and checks
`barkd` for payment settlement. For Arkade addresses, it uses the Arkade Rust SDK
receive-for-user flow to create reverse-swap invoices and claim the resulting
VHTLC output to the recipient Arkade address.

Invoices, SDK swap state, and optional Nostr zap requests are persisted in
Postgres.

## Features

- LNURL-pay metadata at `/.well-known/lnurlp/:address`
- Invoice generation at `/get-invoice/:address`
- Bark address support through `barkd`
- Arkade address support through the Arkade SDK receive-for-user branch
- Optional Nostr zap request storage
- Postgres persistence through Diesel migrations
- Settlement verification at `/verify/:desc_hash/:payment_hash`

## Prerequisites

- Rust toolchain
- Postgres
- Diesel CLI with Postgres support
- A reachable `barkd` REST API
- A reachable Arkade server
- A reachable Boltz endpoint compatible with Arkade reverse swaps
- An Arkade BIP32 xpriv used to claim reverse-swap VHTLCs
- A Nostr `nsec` key for zap metadata

Install Diesel CLI if needed:

```sh
cargo install diesel_cli --no-default-features --features postgres
```

## Configuration

The binary reads command-line flags and environment variables. It also loads a
local `.env` file on startup.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `LNURL_PG_URL` | yes | | Postgres connection string used by the application |
| `LNURL_NSEC` | yes | | Nostr secret key used for zap support |
| `LNURL_BARKD_URL` | yes | | Base URL for the `barkd` REST API |
| `LNURL_BARKD_TOKEN` | no | | Bearer token for the `barkd` REST API |
| `LNURL_DISABLE_ARKADE` | no | `false` | Disable Arkade address support |
| `LNURL_ARKADE_XPRIV` | yes, unless Arkade is disabled | | BIP32 xpriv used by the Arkade SDK to claim reverse-swap VHTLCs |
| `LNURL_ARKADE_SERVER_URL` | yes, unless Arkade is disabled | | Arkade server URL used by the Arkade SDK |
| `LNURL_ARKADE_BOLTZ_URL` | yes, unless Arkade is disabled | | Boltz URL used by the Arkade SDK |
| `LNURL_ARKADE_ESPLORA_URL` | no | `https://mempool.space/api` | Esplora URL used by the Arkade SDK wallet implementation |
| `LNURL_ARKADE_INVOICE_EXPIRY_SECS` | no | | Optional Arkade-generated invoice expiry in seconds |
| `LNURL_BIND` | no | `0.0.0.0` | HTTP bind address |
| `LNURL_PORT` | no | `3000` | HTTP port |
| `LNURL_NETWORK` | no | `bitcoin` | Bitcoin network |
| `LNURL_MIN_SENDABLE` | no | `1000` | Minimum LNURL amount in millisatoshis |
| `LNURL_MAX_SENDABLE` | no | `11000000000` | Maximum LNURL amount in millisatoshis |
| `LNURL_DOMAIN` | no | `localhost:3000` | Public domain used in LNURL callbacks and Lightning addresses |

Arkade addresses require a minimum amount of `333000` millisatoshis.

Example `.env`:

```env
LNURL_PG_URL=postgres://postgres:postgres@127.0.0.1:5432/arkzap_me
LNURL_NSEC=nsec...
LNURL_BARKD_URL=http://127.0.0.1:3535
LNURL_BARKD_TOKEN=
LNURL_ARKADE_XPRIV=xprv...
LNURL_ARKADE_SERVER_URL=http://127.0.0.1:7070
LNURL_ARKADE_BOLTZ_URL=http://127.0.0.1:9001
LNURL_ARKADE_ESPLORA_URL=http://127.0.0.1:3002
LNURL_DOMAIN=example.com
LNURL_PORT=3000
```

Set `LNURL_DISABLE_ARKADE=true` or pass `--disable-arkade` to run with Bark-only
address support. Arkade addresses will be rejected and the Arkade SDK client will
not be initialized.

For Diesel CLI commands, also set `DATABASE_URL` to the same Postgres URL:

```sh
export DATABASE_URL="$LNURL_PG_URL"
```

## Database Setup

Create the database, then run migrations:

```sh
createdb arkzap_me
diesel migration run
```

The migrations create Bark invoice/zap tables, Arkade invoice/zap tables, and
an Arkade SDK swap-storage table.

## Running

```sh
cargo run
```

By default the server listens on `0.0.0.0:3000`.

You can also pass configuration as flags:

```sh
cargo run -- \
  --pg-url postgres://postgres:postgres@127.0.0.1:5432/arkzap_me \
  --nsec nsec... \
  --barkd-url http://127.0.0.1:3535 \
  --arkade-xpriv xprv... \
  --arkade-server-url http://127.0.0.1:7070 \
  --arkade-boltz-url http://127.0.0.1:9001 \
  --domain example.com
```

For Bark-only mode, omit the Arkade options and pass `--disable-arkade`.

## API

### Health Check

```http
GET /health-check
```

Returns a simple health response:

```json
{
  "status": "pass",
  "version": "0"
}
```

Any valid Bark or Arkade address can receive payments at
`<address>@example.com` when `LNURL_DOMAIN=example.com`.

### LNURL-Pay Metadata

```http
GET /.well-known/lnurlp/:address
```

Returns the LNURL-pay callback, amount limits, metadata, and Nostr zap support
information for the Bark or Arkade address.

### Generate Invoice

```http
GET /get-invoice/:address?amount=1000
```

`amount` is required and is denominated in millisatoshis. Invoices are generated
for whole sats, so the amount must be divisible by `1000`. Arkade addresses
require at least `333000` millisatoshis.

Optional query parameters:

- `comment`: LNURL-pay comment, up to 100 characters
- `nostr`: serialized Nostr zap request event

Successful responses include a BOLT11 invoice in the `pr` field and a `verify`
URL for checking settlement status.

### Verify Invoice

```http
GET /verify/:desc_hash/:payment_hash
```

Returns `settled: true` and the payment `preimage` once the Bark or Arkade
backend has revealed and stored the preimage for the invoice. Pending, expired,
or cancelled invoices return `settled: false`.

## Development

Run the test suite:

```sh
cargo test
```

Database-backed migration/model tests are enabled when `LNURL_TEST_DATABASE_URL`
points at a Postgres database the test process can create schemas in:

```sh
LNURL_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/arkzap_me_test cargo test
```

These tests create and drop isolated temporary schemas inside that database.

Format the code:

```sh
cargo fmt
```

Check the project:

```sh
cargo check
```
