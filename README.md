# Annil Serverless

A implementation of [annil](https://book.anni.rs/05.audio-library/01.protocol.html) with shuttle.

## State

- [ ] Authorization
- [x] Info
  - [x] /info
- [ ] Available albums
  - [x] /albums
  - [ ] Add ETag in response
- [ ] Resource Distribution
  - [ ] /{album_id}/{disc_id}/{track_id}
    - [x] get
    - [x] head
    - [ ] respect `quality`
    - [ ] support HTTP range requests
  - [x] /{album_id}/{disc_id}/cover
  - [x] /{album_id}/cover 
- [ ] Admin
  - [x] /admin/reload
  - [ ] /admin/sign