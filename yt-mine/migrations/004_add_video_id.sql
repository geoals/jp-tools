ALTER TABLE mining_jobs ADD COLUMN video_id TEXT;
CREATE INDEX IF NOT EXISTS idx_mining_jobs_video_id ON mining_jobs(video_id);
