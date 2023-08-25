# Annil Serverless

An implementation of [annil](https://book.anni.rs/05.audio-library/01.protocol.html) on shuttle.

This implementation reuses most of [the official one](https://github.com/ProjectAnni/anni/tree/master/annil), but is tailored to [OneDrive backend](https://github.com/snylonue/anni-provider-od).

## State

- [x] Authorization
- [x] Info
  - [x] /info
- [x] Available albums
  - [x] /albums
  - [x] Add ETag in response
- [ ] Resource Distribution
  - [ ] /{album_id}/{disc_id}/{track_id}
    - [x] get
    - [x] head
    - [ ] respect `quality`
    - [x] support HTTP range requests(thanks to onedrive)
  - [x] /{album_id}/{disc_id}/cover
  - [x] /{album_id}/cover
- [x] Admin
  - [x] /admin/reload
  - [x] /admin/sign
