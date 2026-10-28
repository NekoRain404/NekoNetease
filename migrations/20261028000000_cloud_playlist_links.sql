-- The pairing between a cloud playlist and the local playlist it syncs into,
-- plus the cloud snapshot the two sides were reconciled against last.
--
-- Without the snapshot a diff compares two sets and cannot tell a track the
-- user added locally from one the cloud never carried, so the first sync would
-- delete it. The membership table is what makes a three-way diff possible: a
-- local id missing from both the cloud and the snapshot is a local addition,
-- while one present in the snapshot and gone from the cloud is a cloud removal.
--
-- The snapshot records the *cloud* side, not the local one. A track the user
-- added locally is then absent from the base and reads as a local addition on
-- every later pass rather than as a cloud removal the sync would undo.
--
-- Both names are stored for the same reason applied to renames: a playlist
-- linked under the name it already had would otherwise read as locally renamed
-- on every pass, and a later cloud rename could never be told from it.
CREATE TABLE IF NOT EXISTS cloud_playlist_links (
    cloud_playlist_id INTEGER PRIMARY KEY,
    local_playlist_id INTEGER NOT NULL UNIQUE REFERENCES playlists (id) ON DELETE CASCADE,
    -- The cloud name at the last apply, so a later pass can ask which side moved.
    cloud_name TEXT NOT NULL,
    local_name TEXT NOT NULL,
    synced_at TEXT NOT NULL
);

-- The cloud membership as of the last apply, in cloud order. Keyed to the link
-- rather than to a local track so deleting either side cascades the snapshot
-- with it.
CREATE TABLE IF NOT EXISTS cloud_playlist_link_tracks (
    cloud_playlist_id INTEGER NOT NULL
        REFERENCES cloud_playlist_links (cloud_playlist_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    local_track_id INTEGER NOT NULL,
    PRIMARY KEY (cloud_playlist_id, local_track_id)
);

CREATE INDEX IF NOT EXISTS idx_cloud_playlist_link_tracks_position
    ON cloud_playlist_link_tracks (cloud_playlist_id, position);
