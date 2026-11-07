//! 客户端本地存储(rusqlite)。
//!
//! 会话列表/未读数全部本地计算:未读 = rowid 大于已读游标、非本人、未撤回的消息数。
//! 消息以 uuid 为主键幂等插入(gossip 重复投递与 HTTP 历史补齐都安全)。

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::types::{ConvType, Conversation, LocalMessage, MemberInfo, Message, MsgType, User};

pub struct ClientStore {
    conn: Mutex<Connection>,
}

/// 会话列表行(带派生字段)
#[derive(Debug, Clone)]
pub struct ConversationView {
    pub conversation: Conversation,
    pub pinned: bool,
    pub last_message: Option<Message>,
    pub unread: i64,
}

impl ClientStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    // -----------------------------------------------------------------
    // KV(settings)
    // -----------------------------------------------------------------

    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let v = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(v)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // 用户
    // -----------------------------------------------------------------

    pub fn upsert_user(&self, u: &User) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users(id, ymid, name, phone, email, avatar, is_online)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               ymid = excluded.ymid, name = excluded.name, phone = excluded.phone,
               email = excluded.email, avatar = excluded.avatar, is_online = excluded.is_online",
            params![u.id, u.ymid, u.name, u.phone, u.email, u.avatar, u.is_online as i64],
        )?;
        Ok(())
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ymid, name, phone, email, avatar, is_online FROM users ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], row_to_user)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -----------------------------------------------------------------
    // 会话
    // -----------------------------------------------------------------

    /// 用服务器返回的会话列表刷新本地(保留本地 pinned)。
    pub fn replace_conversations(&self, convs: &[Conversation]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for c in convs {
            conn.execute(
                "INSERT INTO conversations(id, title, conv_type, topic_id, created_by, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title, conv_type = excluded.conv_type,
                   topic_id = excluded.topic_id, updated_at = excluded.updated_at",
                params![c.id, c.title, c.conv_type.as_str(), c.topic_id, c.created_by, c.updated_at],
            )?;
            conn.execute(
                "DELETE FROM conversation_members WHERE conversation_id = ?1",
                params![c.id],
            )?;
            for m in &c.members {
                conn.execute(
                    "INSERT OR IGNORE INTO conversation_members(conversation_id, user_id, name, role)
                     VALUES(?1, ?2, ?3, ?4)",
                    params![c.id, m.user_id, m.name, m.role],
                )?;
            }
        }
        Ok(())
    }

    /// 单个会话(成员齐备);本地库无则 None
    pub fn get_conversation(&self, conv_id: i64) -> Result<Option<Conversation>> {
        let conn = self.conn.lock().unwrap();
        let Some((id, title, conv_type, topic_id, created_by, updated_at)) = conn
            .query_row(
                "SELECT id, title, conv_type, topic_id, created_by, updated_at FROM conversations WHERE id = ?1",
                params![conv_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(
            "SELECT user_id, name, role FROM conversation_members WHERE conversation_id = ?1 ORDER BY user_id",
        )?;
        let members = stmt
            .query_map(params![conv_id], |r| {
                Ok(MemberInfo { user_id: r.get(0)?, name: r.get(1)?, role: r.get(2)? })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(Conversation {
            id,
            title,
            conv_type: ConvType::from_str(&conv_type),
            topic_id,
            created_by,
            updated_at,
            members,
        }))
    }

    pub fn set_pinned(&self, conv_id: i64, pinned: bool) -> Result<()> {        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE conversations SET pinned = ?2 WHERE id = ?1",
            params![conv_id, pinned as i64],
        )?;
        Ok(())
    }

    pub fn get_pinned(&self, conv_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let v = conn
            .query_row(
                "SELECT pinned FROM conversations WHERE id = ?1",
                params![conv_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0) != 0)
    }

    /// 会话列表:置顶最前, 其余按最后消息时间(无消息用 updated_at)倒序
    pub fn list_conversations(&self, my_id: i64) -> Result<Vec<ConversationView>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, conv_type, topic_id, created_by, pinned, updated_at
             FROM conversations",
        )?;
        let mut rows: Vec<(Conversation, bool)> = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let title: String = r.get(1)?;
                let conv_type: String = r.get(2)?;
                let topic_id: String = r.get(3)?;
                let created_by: i64 = r.get(4)?;
                let pinned: i64 = r.get(5)?;
                let updated_at: String = r.get(6)?;
                Ok((
                    Conversation {
                        id,
                        title,
                        conv_type: ConvType::from_str(&conv_type),
                        topic_id,
                        created_by,
                        updated_at,
                        members: Vec::new(),
                    },
                    pinned != 0,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut views = Vec::with_capacity(rows.len());
        for (mut conv, pinned) in rows.drain(..) {
            // 成员
            {
                let mut mstmt = conn.prepare(
                    "SELECT user_id, name, role FROM conversation_members WHERE conversation_id = ?1 ORDER BY user_id",
                )?;
                conv.members = mstmt
                    .query_map(params![conv.id], |r| {
                        Ok(MemberInfo {
                            user_id: r.get(0)?,
                            name: r.get(1)?,
                            role: r.get(2)?,
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            }
            // 最后一条消息 + 未读
            let last = query_last_message(&conn, conv.id)?;
            let unread: i64 = conn.query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE conversation_id = ?1 AND is_recalled = 0 AND sender_id <> ?2
                   AND rowid > COALESCE((SELECT last_read_rowid FROM read_cursors WHERE conversation_id = ?1), 0)",
                params![conv.id, my_id],
                |r| r.get(0),
            )?;
            views.push(ConversationView { conversation: conv, pinned, last_message: last, unread });
        }
        views.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                let ta = a.last_message.as_ref().map(|m| m.created_at_ms).unwrap_or(0);
                let tb = b.last_message.as_ref().map(|m| m.created_at_ms).unwrap_or(0);
                tb.cmp(&ta).then_with(|| b.conversation.updated_at.cmp(&a.conversation.updated_at))
            })
        });
        Ok(views)
    }

    // -----------------------------------------------------------------
    // 消息
    // -----------------------------------------------------------------

    /// 幂等插入;返回是否真的新插入(gossip 去重用)
    pub fn insert_message(&self, m: &Message, server_seq: Option<i64>) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "INSERT OR IGNORE INTO messages(
                id, conversation_id, sender_id, sender_name, msg_type, content,
                reply_to, is_recalled, server_seq, attachments, created_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.id.to_string(),
                m.conversation_id,
                m.sender_id,
                m.sender_name,
                m.msg_type.as_str(),
                m.content,
                m.reply_to.map(|u| u.to_string()),
                m.is_recalled as i64,
                server_seq,
                serde_json::to_string(&m.attachments).unwrap_or_else(|_| "[]".into()),
                m.created_at_ms,
            ],
        )?;
        Ok(n > 0)
    }

    /// 历史补齐:回填 server_seq(仅当该消息已在且 seq 为空)
    pub fn fill_server_seq(&self, msg_id: &str, server_seq: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET server_seq = ?2 WHERE id = ?1 AND server_seq IS NULL",
            params![msg_id, server_seq],
        )?;
        Ok(())
    }

    pub fn mark_recalled(&self, conversation_id: i64, message_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET is_recalled = 1 WHERE conversation_id = ?1 AND id = ?2",
            params![conversation_id, message_id],
        )?;
        Ok(())
    }

    /// 升序返回一页消息;`before_rowid` 为 None 时取最新一页
    pub fn list_messages(
        &self,
        conversation_id: i64,
        limit: usize,
        before_rowid: Option<i64>,
    ) -> Result<Vec<LocalMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT rowid, id, conversation_id, sender_id, sender_name, msg_type, content,
                    reply_to, is_recalled, server_seq, attachments, created_at_ms
             FROM messages WHERE conversation_id = ?1 AND (?2 IS NULL OR rowid < ?2)
             ORDER BY rowid DESC LIMIT ?3",
        )?;
        let mut rows = stmt
            .query_map(
                params![conversation_id, before_rowid, limit as i64],
                |r| row_to_local_message(r),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.reverse(); // 存储倒序取, 展示升序
        Ok(rows)
    }

    pub fn get_message(&self, message_id: &str) -> Result<Option<LocalMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT rowid, id, conversation_id, sender_id, sender_name, msg_type, content,
                    reply_to, is_recalled, server_seq, attachments, created_at_ms
             FROM messages WHERE id = ?1",
        )?;
        let row = stmt.query_row(params![message_id], row_to_local_message).optional()?;
        Ok(row)
    }

    /// 该会话已知的最大服务器序号(历史补齐游标;无则 0)
    pub fn max_server_seq(&self, conversation_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let v: Option<i64> = conn
            .query_row(
                "SELECT MAX(server_seq) FROM messages WHERE conversation_id = ?1",
                params![conversation_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(v.unwrap_or(0))
    }

    /// 标记已读(游标推到该会话最大 rowid), 返回新游标
    pub fn mark_read(&self, conversation_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let max_rowid: i64 = conn.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM messages WHERE conversation_id = ?1",
            params![conversation_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO read_cursors(conversation_id, last_read_rowid) VALUES(?1, ?2)
             ON CONFLICT(conversation_id) DO UPDATE SET last_read_rowid = excluded.last_read_rowid",
            params![conversation_id, max_rowid],
        )?;
        Ok(max_rowid)
    }

    pub fn last_read_rowid(&self, conversation_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let v: Option<i64> = conn
            .query_row(
                "SELECT last_read_rowid FROM read_cursors WHERE conversation_id = ?1",
                params![conversation_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0))
    }
}

// ---------------------------------------------------------------------------
// 行映射
// ---------------------------------------------------------------------------

fn row_to_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        ymid: r.get(1)?,
        name: r.get(2)?,
        phone: r.get(3)?,
        email: r.get(4)?,
        avatar: r.get(5)?,
        is_online: r.get::<_, i64>(6)? != 0,
    })
}

fn row_to_local_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<LocalMessage> {
    let id: String = r.get(1)?;
    let attachments_json: String = r.get(10)?;
    let msg_type: String = r.get(5)?;
    Ok(LocalMessage {
        rowid: r.get(0)?,
        server_seq: r.get(9)?,
        message: Message {
            id: uuid::Uuid::parse_str(&id).unwrap_or_default(),
            conversation_id: r.get(2)?,
            sender_id: r.get(3)?,
            sender_name: r.get(4)?,
            msg_type: MsgType::from_str(&msg_type),
            content: r.get(6)?,
            reply_to: r.get::<_, Option<String>>(7)?.and_then(|s| uuid::Uuid::parse_str(&s).ok()),
            is_recalled: r.get::<_, i64>(8)? != 0,
            attachments: serde_json::from_str(&attachments_json).unwrap_or_default(),
            created_at_ms: r.get(11)?,
        },
    })
}

fn query_last_message(conn: &Connection, conversation_id: i64) -> Result<Option<Message>> {
    let mut stmt = conn.prepare(
        "SELECT rowid, id, conversation_id, sender_id, sender_name, msg_type, content,
                reply_to, is_recalled, server_seq, attachments, created_at_ms
         FROM messages WHERE conversation_id = ?1 AND is_recalled = 0
         ORDER BY rowid DESC LIMIT 1",
    )?;
    let row = stmt
        .query_row(params![conversation_id], |r| {
            Ok(row_to_local_message(r)?.message)
        })
        .optional()?;
    Ok(row)
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id        INTEGER PRIMARY KEY,
    ymid      TEXT UNIQUE NOT NULL,
    name      TEXT NOT NULL,
    phone     TEXT,
    email     TEXT,
    avatar    TEXT,
    is_online INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS conversations (
    id         INTEGER PRIMARY KEY,
    title      TEXT NOT NULL DEFAULT '',
    conv_type  TEXT NOT NULL,
    topic_id   TEXT NOT NULL,
    created_by INTEGER NOT NULL,
    pinned     INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS conversation_members (
    conversation_id INTEGER NOT NULL,
    user_id         INTEGER NOT NULL,
    name            TEXT NOT NULL DEFAULT '',
    role            TEXT NOT NULL DEFAULT 'member',
    PRIMARY KEY (conversation_id, user_id)
);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id INTEGER NOT NULL,
    sender_id       INTEGER NOT NULL,
    sender_name     TEXT NOT NULL DEFAULT '',
    msg_type        TEXT NOT NULL DEFAULT 'text',
    content         TEXT NOT NULL DEFAULT '',
    reply_to        TEXT,
    is_recalled     INTEGER NOT NULL DEFAULT 0,
    server_seq      INTEGER,
    attachments     TEXT NOT NULL DEFAULT '[]',
    created_at_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msg_conv ON messages(conversation_id);

CREATE TABLE IF NOT EXISTS read_cursors (
    conversation_id INTEGER PRIMARY KEY,
    last_read_rowid INTEGER NOT NULL DEFAULT 0
);
"#;
