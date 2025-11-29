// @generated automatically by Diesel CLI.

diesel::table! {
    recents (id) {
        id -> Integer,
        book_path -> Text,
        update_at -> BigInt,
        page -> Integer,
        page_count -> Integer,
        create_at -> BigInt,
        crop -> Integer,
        reflow -> Integer,
        scroll_ori -> Integer,
        zoom -> Double,
        scroll_x -> Integer,
        scroll_y -> Integer,
        name -> Text,
        ext -> Text,
        size -> BigInt,
        read_times -> Integer,
        progress -> BigInt,
        favorited -> Integer,
        in_recent -> Integer,
    }
}
