// @generated automatically by Diesel CLI.

diesel::table! {
    arkade_invoice (id) {
        id -> Int4,
        recipient_address -> Text,
        #[max_length = 2048]
        bolt11 -> Varchar,
        amount_msats -> Int8,
        #[max_length = 64]
        payment_hash -> Nullable<Varchar>,
        #[max_length = 64]
        preimage -> Varchar,
        swap_id -> Text,
        #[max_length = 100]
        lnurlp_comment -> Nullable<Varchar>,
        state -> Int4,
        created_at -> Timestamp,
        expires_at -> Nullable<Timestamp>,
        settled_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    arkade_swap_storage (swap_type, swap_id) {
        swap_id -> Text,
        swap_type -> Text,
        data -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    arkade_zaps (id) {
        id -> Int4,
        request -> Text,
        #[max_length = 64]
        event_id -> Nullable<Varchar>,
    }
}

diesel::table! {
    custom_address_invoice (id) {
        id -> Int4,
        #[max_length = 32]
        name -> Varchar,
        ark_address -> Text,
        auth_message -> Text,
        #[max_length = 128]
        signature -> Varchar,
        fee_receive_address -> Text,
        #[max_length = 2048]
        bolt11 -> Varchar,
        amount_msats -> Int8,
        #[max_length = 64]
        payment_hash -> Nullable<Varchar>,
        #[max_length = 64]
        preimage -> Varchar,
        state -> Int4,
        created_at -> Timestamp,
        expires_at -> Nullable<Timestamp>,
        settled_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    custom_addresses (id) {
        id -> Int4,
        #[max_length = 32]
        name -> Varchar,
        ark_address -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    invoice (id) {
        id -> Int4,
        ark_address -> Text,
        #[max_length = 2048]
        bolt11 -> Varchar,
        amount_msats -> Int8,
        #[max_length = 64]
        payment_hash -> Nullable<Varchar>,
        #[max_length = 64]
        preimage -> Varchar,
        #[max_length = 100]
        lnurlp_comment -> Nullable<Varchar>,
        state -> Int4,
        created_at -> Timestamp,
        expires_at -> Nullable<Timestamp>,
        settled_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    zaps (id) {
        id -> Int4,
        request -> Text,
        #[max_length = 64]
        event_id -> Nullable<Varchar>,
    }
}

diesel::joinable!(arkade_zaps -> arkade_invoice (id));
diesel::joinable!(zaps -> invoice (id));

diesel::allow_tables_to_appear_in_same_query!(
    arkade_invoice,
    arkade_swap_storage,
    arkade_zaps,
    custom_address_invoice,
    custom_addresses,
    invoice,
    zaps,
);
