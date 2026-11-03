ALTER TABLE users ADD COLUMN locale TEXT CHECK (locale IN ('en', 'zh-CN'));
