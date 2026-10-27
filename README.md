# arkzap-me

`arkzap-me` is a small LNURL-pay server for Bark and Arkade. It gives every
supported Ark address a deterministic Lightning address, serves LNURL-pay
metadata, and generates BOLT11 invoices for incoming payments.

For Bark addresses, the service asks `barkd` to generate invoices and checks
`barkd` for payment settlement. For Arkade addresses, it uses the Arkade Rust SDK
receive-for-user flow to create reverse-swap invoices and claim the resulting
VHTLC output to the recipient Arkade address.

Users can also buy custom Lightning address names backed by their Bark Ark
address. Ownership is authenticated with Bark address message signatures, and
each name activates after its fee invoice is paid to the service Bark wallet.

Invoices, custom names, SDK swap state, and optional Nostr zap requests are persisted in
Postgres.

## Features

- LNURL-pay metadata at `/.well-known/lnurlp/:address`
- Invoice generation at `/get-invoice/:address`
- Bark address support through `barkd`
- Paid custom Lightning address names for Bark addresses
- Arkade address support through the Arkade SDK receive-for-user branch
- Nostr zaps: validates zap requests and publishes zap receipts once paid
- Postgres persistence through Diesel migrations
- Settlement verification at `/verify/:address/:payment_hash`

## Prerequisites

- Rust toolchain
- Postgres
- Diesel CLI with Postgres support
- A reachable `barkd` v0.6.2 or later REST API
- A reachable Arkade server
- A reachable Boltz endpoint compatible with Arkade reverse swaps
- An Arkade BIP32 xpriv used to claim reverse-swap VHTLCs
- A Nostr `nsec` key used to sign zap receipts

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
| `LNURL_NSEC` | yes | | Nostr secret key used to sign zap receipts |
| `LNURL_BARKD_URL` | yes | | Base URL for the `barkd` REST API |
| `LNURL_BARKD_TOKEN` | no | | Bearer token for `barkd`. Required unless `barkd` uses `--no-auth` on a loopback address |
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
| `LNURL_CUSTOM_ADDRESS_FEE_SATS` | no | `10000` | Fee charged to activate each custom Lightning address name |
| `LNURL_DOMAIN` | no | `localhost:3000` | Public domain used in LNURL callbacks and Lightning addresses |

Arkade addresses require a minimum amount of `333000` millisatoshis.

Bark v0.6.2 does not read `BARKD_NO_AUTH`.

If `barkd` listens on a loopback address, start it with `--no-auth`. For all
other deployments, set `LNURL_BARKD_TOKEN` to the bearer token from `barkd`.

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

## NixOS

The flake provides the server package, a development shell, and a NixOS module.
Import the module from your system flake and enable the service:

```nix
{
  inputs.arkzap-me.url = "github:benthecarman/arkzap-me";

  outputs = { nixpkgs, arkzap-me, ... }: {
    nixosConfigurations.my-server = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        arkzap-me.nixosModules.default
        ({ ... }: {
          services.arkzap-me = {
            enable = true;
            environmentFiles = [ "/run/secrets/arkzap-me.env" ];
            environment = {
              LNURL_DOMAIN = "example.com";
              LNURL_BARKD_URL = "http://127.0.0.1:3535";
              LNURL_ARKADE_SERVER_URL = "http://127.0.0.1:7070";
              LNURL_ARKADE_BOLTZ_URL = "http://127.0.0.1:9001";
            };
            # Leave this false when a reverse proxy is on the same host.
            openFirewall = false;
          };
        })
      ];
    };
  };
}
```

Put secrets in the environment file. Keep this file out of the Nix store; for
example, create it through `sops-nix` or `agenix`:

```env
LNURL_PG_URL=postgresql://arkzap:password@127.0.0.1/arkzap-me
LNURL_NSEC=nsec...
LNURL_ARKADE_XPRIV=xprv...
LNURL_BARKD_TOKEN=...
```

The module does not create or configure PostgreSQL. `LNURL_PG_URL` can point to
a local or remote database. The application runs embedded database migrations
when the service starts.

Build or run the package directly with `nix build` or `nix run`. Enter the Rust
development environment with `nix develop`.

## API

### Service Info

```http
GET /
```

Returns an HTML information page for arkzap.me.

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

- `nostr`: serialized NIP-57 zap request event. The request must be signed,
  have exactly one `p` tag, list at least one public relay in its `relays`
  tag, and match `amount` if it has an `amount` tag. After the invoice is
  paid, a zap receipt signed with `LNURL_NSEC` is published to up to 10 of the
  listed relays. Failed publishes are retried for 24 hours.

Successful responses include a BOLT11 invoice in the `pr` field and a `verify`
URL for checking settlement status.

### Custom Lightning addresses

Custom names map a short local-part like `alice` to a Bark Ark address. Names are
3 to 32 characters and may contain lowercase letters, numbers, hyphens, and
underscores. Arkade addresses are not supported for custom names.

Get the ownership message:

```http
GET /custom-addresses/auth-message?name=alice&arkAddress=ark...
```

Use Bark v0.6.2 or later to sign this message.

The response includes these signing methods:

- `signing.cliCommand` is an argument list for the Bark CLI.
- `signing.barkdRequest` contains the method, path, and JSON body for the
  `barkd` REST API.

With the Bark CLI, pass each list item as a separate argument. Do not change
the message item.

The Bark CLI uses this command form:

```sh
bark message sign "<message>" "<ark-address>"
```

The `barkd` REST API uses this request form:

```http
POST /api/v1/message/sign
Content-Type: application/json
Authorization: Bearer <token>

{
  "message": "<message>",
  "address": "<ark-address>"
}
```

Copy the `signature` value from the Bark JSON response.

Submit the Bark address signature to create the fee invoice:

```http
POST /custom-addresses
Content-Type: application/json

{
  "name": "alice",
  "arkAddress": "ark...",
  "signature": "..."
}
```

The response includes the BOLT11 fee invoice and a purchase `id`. Poll the
purchase until it is active:

```http
GET /custom-addresses/:id
```

Name reservations are set to expire after at most one hour. If Bark has accepted
the Lightning payment, the name stays reserved until that payment resolves,
even after invoice expiry. The service checks for Lightning and Ark fee
payments before it releases an expired reservation. If the payment status cannot
be checked, the reservation stays pending until a later check succeeds.

Once active, `<name>@example.com` works through the normal LNURL-pay and verify
endpoints for life. The same Bark address can sign a new message and pay another
fee to claim another available name.

### Verify Invoice

```http
GET /verify/:address/:payment_hash
```

Returns `settled: true` and the payment `preimage` once the Bark or Arkade
backend has revealed and stored the preimage for the invoice. Pending and
cancelled invoices return `settled: false`. Accepted Bark payments stay pending
through invoice expiry and can settle later. Unpaid Bark invoices are cancelled
after expiry once Bark reports an awaiting payment or no receive record.

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
