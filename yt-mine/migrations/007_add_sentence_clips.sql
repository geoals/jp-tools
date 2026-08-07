ALTER TABLE mining_sentences ADD COLUMN source TEXT NOT NULL DEFAULT 'captions';
ALTER TABLE mining_sentences ADD COLUMN clip_path TEXT;
ALTER TABLE mining_sentences ADD COLUMN clip_audio_path TEXT;
ALTER TABLE mining_sentences ADD COLUMN clip_start REAL;

-- Everything that existed before this column did came from whisper over the
-- whole video, which is what the job's own audio_path still points at.
UPDATE mining_sentences SET source = 'whisper';
