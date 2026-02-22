CREATE TABLE IF NOT EXISTS mining_jobs (
    id INTEGER PRIMARY KEY,
    youtube_url TEXT NOT NULL,
    video_title TEXT,
    audio_path TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    error_message TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS mining_sentences (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES mining_jobs(id),
    text TEXT NOT NULL,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mining_sentences_job ON mining_sentences(job_id);
