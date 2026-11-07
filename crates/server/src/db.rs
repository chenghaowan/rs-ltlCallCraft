//! 服务器 SQLite 存储(rusqlite, Mutex 串行化;本场景写放大很小, 足够)。
//!
//! 表结构语义对齐 ltl-manage(users.ymid 锚点、单聊幂等、消息软删=撤回),
//! 消息主键为发送方生成的 uuid, seq 为服务器历史递增序号(历史补齐游标)。

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use ltl_chat_core::error::{LtlError, Result};
use ltl_chat_core::types::{
    ConvType, Conversation, MemberInfo, Message, MsgType, UserId,
};

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: UserId,
    pub ymid: String,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub avatar: Option<String>,
    pub ym_auth_token: Option<String>,
    pub is_active: bool,
    pub last_active_ms: i64,
}

impl UserRow {
    pub fn to_user(&self, online_threshold_ms: i64, now_ms: i64) -> ltl_chat_core::types::User {
        ltl_chat_core::types::User {
            id: self.id,
            ymid: self.ymid.clone(),
            name: self.name.clone(),
            phone: self.phone.clone(),
            email: self.email.clone(),
            avatar: self.avatar.clone(),
            is_online: self.is_active && now_ms - self.last_active_ms < online_threshold_ms,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AttachmentRow {
    pub id: i64,
    pub file_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub file_type: String,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    // -----------------------------------------------------------------
    // 用户(登记 = 首次登录 upsert)
    // -----------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_user_by_ymid(
        &self,
        ymid: &str,
        name: &str,
        email: Option<&str>,
        avatar: Option<&str>,
        phone: Option<&str>,
        ym_token: Option<&str>,
        ym_refresh: Option<&str>,
        now_ms: i64,
    ) -> Result<UserRow> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users(ymid, name, email, avatar, phone, ym_auth_token, ym_auth_refresh_token,
                               is_active, last_active_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)
             ON CONFLICT(ymid) DO UPDATE SET
               name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE users.name END,
               email = COALESCE(excluded.email, users.email),
               avatar = COALESCE(excluded.avatar, users.avatar),
               phone = COALESCE(excluded.phone, users.phone),
               ym_auth_token = COALESCE(excluded.ym_auth_token, users.ym_auth_token),
               ym_auth_refresh_token = COALESCE(excluded.ym_auth_refresh_token, users.ym_auth_refresh_token),
               is_active = 1,
               last_active_ms = excluded.last_active_ms",
            params![ymid, name, email, avatar, phone, ym_token, ym_refresh, now_ms],
        )?;
        let id: i64 = conn.query_row("SELECT id FROM users WHERE ymid = ?1", params![ymid], |r| r.get(0))?;
        self.get_user(&conn, id)?
            .ok_or_else(|| LtlError::Other("用户登记失败".into()))
    }

    fn get_user(&self, conn: &Connection, id: UserId) -> Result<Option<UserRow>> {
        let row = conn
            .query_row(
                "SELECT id, ymid, name, phone, email, avatar, ym_auth_token, is_active, last_active_ms
                 FROM users WHERE id = ?1",
                params![id],
                |r| row_to_user(r),
            )
            .optional()?;
        Ok(row)
    }

    pub fn user(&self, id: UserId) -> Result<Option<UserRow>> {
        let conn = self.conn.lock().unwrap();
        self.get_user(&conn, id)
    }

    pub fn list_users(&self) -> Result<Vec<UserRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ymid, name, phone, email, avatar, ym_auth_token, is_active, last_active_ms
             FROM users ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], row_to_user)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// YM 全员同步:新增登记, 远端缺失的停用
    pub fn sync_roster(&self, list: &[ltl_chat_core::ym::YmListUser], now_ms: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut n = 0;
        for u in list {
            conn.execute(
                "INSERT INTO users(ymid, name, phone, email, is_active, last_active_ms)
                 VALUES(?1, ?2, ?3, ?4, 1, 0)
                 ON CONFLICT(ymid) DO UPDATE SET
                   name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE users.name END,
                   phone = COALESCE(excluded.phone, users.phone),
                   email = COALESCE(excluded.email, users.email),
                   is_active = 1",
                params![u.ymid, u.name, u.phone, u.email],
            )?;
            n += 1;
        }
        if !list.is_empty() {
            let ymids: HashSet<&str> = list.iter().map(|u| u.ymid.as_str()).collect();
            // 远端没有的本地用户停用(demo 用户除外)
            let mut stmt = conn.prepare("SELECT id, ymid FROM users")?;
            let stale: Vec<i64> = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|row| {
                    let (id, ymid) = row.ok()?;
                    if ymids.contains(ymid.as_str()) || ymid.starts_with("demo_") {
                        None
                    } else {
                        Some(id)
                    }
                })
                .collect();
            drop(stmt);
            for id in stale {
                conn.execute("UPDATE users SET is_active = 0 WHERE id = ?1", params![id])?;
            }
        }
        let _ = now_ms;
        Ok(n)
    }

    pub fn touch(&self, id: UserId, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE users SET last_active_ms = ?2 WHERE id = ?1", params![id, now_ms])?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // 会话
    // -----------------------------------------------------------------

    /// 单聊幂等:恰好 2 成员且为单聊类型的既有会话
    pub fn find_single_conversation(&self, a: UserId, b: UserId) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT cm.conversation_id
             FROM conversation_members cm
             JOIN conversations c ON c.id = cm.conversation_id
             WHERE cm.user_id IN (?1, ?2) AND c.conv_type = 'single'
             GROUP BY cm.conversation_id
             HAVING COUNT(*) = 2
             LIMIT 1",
        )?;
        let id = stmt.query_row(params![a, b], |r| r.get::<_, i64>(0)).optional()?;
        Ok(id)
    }

    pub fn create_conversation(
        &self,
        conv_type: ConvType,
        title: Option<&str>,
        creator: UserId,
        member_ids: &[UserId],
        topic_id_hex: &str,
        now_ms: i64,
    ) -> Result<i64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(LtlError::from)?;
        tx.execute(
            "INSERT INTO conversations(title, conv_type, topic_id, created_by, created_at_ms, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![title.unwrap_or(""), conv_type.as_str(), topic_id_hex, creator, now_ms, ""],
        )?;
        let conv_id = tx.last_insert_rowid();
        let mut seen = HashSet::new();
        seen.insert(creator);
        tx.execute(
            "INSERT INTO conversation_members(conversation_id, user_id, role) VALUES(?1, ?2, 'owner')",
            params![conv_id, creator],
        )?;
        for uid in member_ids {
            if *uid == creator || !seen.insert(*uid) {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO conversation_members(conversation_id, user_id, role)
                 VALUES(?1, ?2, 'member')",
                params![conv_id, uid],
            )?;
        }
        tx.commit().map_err(LtlError::from)?;
        Ok(conv_id)
    }

    pub fn add_members(&self, conv_id: i64, member_ids: &[UserId]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for uid in member_ids {
            conn.execute(
                "INSERT OR IGNORE INTO conversation_members(conversation_id, user_id, role)
                 VALUES(?1, ?2, 'member')",
                params![conv_id, uid],
            )?;
        }
        Ok(())
    }

    pub fn conversation(&self, conv_id: i64) -> Result<Option<Conversation>> {
        let conn = self.conn.lock().unwrap();
        self.conversation_inner(&conn, conv_id)
    }

    fn conversation_inner(&self, conn: &Connection, conv_id: i64) -> Result<Option<Conversation>> {
        let Some((id, title, conv_type, topic_id, created_by, updated_at)) = conn
            .query_row(
                "SELECT id, title, conv_type, topic_id, created_by, updated_at
                 FROM conversations WHERE id = ?1",
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
            "SELECT cm.user_id, COALESCE(u.name, ''), cm.role
             FROM conversation_members cm LEFT JOIN users u ON u.id = cm.user_id
             WHERE cm.conversation_id = ?1 ORDER BY cm.user_id",
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

    /// 用户会话列表(按最后消息时间倒序;title 对单聊填对方名)
    pub fn conversations_for_user(&self, user_id: UserId) -> Result<Vec<Conversation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id FROM conversations c
             JOIN conversation_members cm ON cm.conversation_id = c.id
             WHERE cm.user_id = ?1
             ORDER BY (SELECT COALESCE(MAX(created_at_ms), 0) FROM messages m
                       WHERE m.conversation_id = c.id) DESC, c.id DESC",
        )?;
        let ids = stmt
            .query_map(params![user_id], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(mut conv) = self.conversation_inner(&conn, id)? {
                if conv.conv_type == ConvType::Single {
                    conv.title = conv
                        .members
                        .iter()
                        .find(|m| m.user_id != user_id)
                        .map(|m| m.name.clone())
                        .unwrap_or_default();
                }
                // 最后消息时间 → updated_at(客户端排序用)
                conv.updated_at = conn
                    .query_row(
                        "SELECT COALESCE(MAX(created_at_ms), 0) FROM messages WHERE conversation_id = ?1",
                        params![id],
                        |r| r.get::<_, i64>(0),
                    )?
                    .to_string();
                out.push(conv);
            }
        }
        Ok(out)
    }

    pub fn is_member(&self, conv_id: i64, user_id: UserId) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM conversation_members WHERE conversation_id = ?1 AND user_id = ?2",
            params![conv_id, user_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn all_topic_ids(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT topic_id FROM conversations")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -----------------------------------------------------------------
    // 消息
    // -----------------------------------------------------------------

    /// 幂等写入 gossip 收到的消息;返回 seq(重复消息返回已有 seq)
    pub fn insert_message(&self, m: &Message) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT seq FROM messages WHERE id = ?1",
                params![m.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(seq) = existing {
            return Ok(seq);
        }
        conn.execute(
            "INSERT INTO messages(id, conversation_id, sender_id, msg_type, content, reply_to,
                                  is_recalled, attachments, created_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.id.to_string(),
                m.conversation_id,
                m.sender_id,
                m.msg_type.as_str(),
                m.content,
                m.reply_to.map(|u| u.to_string()),
                m.is_recalled as i64,
                serde_json::to_string(&m.attachments).unwrap_or_else(|_| "[]".into()),
                m.created_at_ms,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn mark_recalled(&self, conversation_id: i64, message_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET is_recalled = 1 WHERE conversation_id = ?1 AND id = ?2",
            params![conversation_id, message_id],
        )?;
        Ok(())
    }

    /// 历史分页, sender_name 由 users 表联出:
    /// - `Some(after)`:seq > after,升序(增量补齐)
    /// - `Some(before)`:seq < before,取 limit 条后反转(向上翻页)
    /// - `None, None`:最新 limit 条(反转后升序, 首次同步)
    pub fn history_page(
        &self,
        conversation_id: i64,
        after_seq: Option<i64>,
        before_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<(i64, Message)>> {
        let conn = self.conn.lock().unwrap();
        let (sql_order, edge, edge_val): (&str, &str, i64) = if let Some(after) = after_seq {
            ("ASC", ">", after)
        } else if let Some(before) = before_seq {
            ("DESC", "<", before)
        } else {
            ("DESC", "<", i64::MAX)
        };
        let sql = format!(
            "SELECT m.seq, m.id, m.conversation_id, m.sender_id, COALESCE(u.name, ''), m.msg_type,
                    m.content, m.reply_to, m.is_recalled, m.attachments, m.created_at_ms
             FROM messages m LEFT JOIN users u ON u.id = m.sender_id
             WHERE m.conversation_id = ?1 AND m.seq {edge} ?2
             ORDER BY m.seq {sql_order} LIMIT ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt
            .query_map(params![conversation_id, edge_val, limit as i64], map_history_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if sql_order == "DESC" {
            rows.reverse();
        }
        Ok(rows)
    }

    // -----------------------------------------------------------------
    // 附件(上传即登记;消息只带元数据)
    // -----------------------------------------------------------------

    pub fn insert_attachment(&self, file_name: &str, file_path: &str, file_size: i64, file_type: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO attachments(message_id, file_name, file_path, file_size, file_type)
             VALUES(NULL, ?1, ?2, ?3, ?4)",
            params![file_name, file_path, file_size, file_type],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn attachment(&self, id: i64) -> Result<Option<AttachmentRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT id, file_name, file_path, file_size, file_type FROM attachments WHERE id = ?1",
                params![id],
                |r| {
                    Ok(AttachmentRow {
                        id: r.get(0)?,
                        file_name: r.get(1)?,
                        file_path: r.get(2)?,
                        file_size: r.get(3)?,
                        file_type: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

fn map_history_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, Message)> {
    let seq: i64 = r.get(0)?;
    let id: String = r.get(1)?;
    let msg_type: String = r.get(5)?;
    let msg = Message {
        id: uuid::Uuid::parse_str(&id).unwrap_or_default(),
        conversation_id: r.get(2)?,
        sender_id: r.get(3)?,
        sender_name: r.get(4)?,
        msg_type: MsgType::from_str(&msg_type),
        content: r.get(6)?,
        reply_to: r
            .get::<_, Option<String>>(7)?
            .and_then(|s| uuid::Uuid::parse_str(&s).ok()),
        is_recalled: r.get::<_, i64>(8)? != 0,
        attachments: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
        created_at_ms: r.get(10)?,
    };
    Ok((seq, msg))
}

fn row_to_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<UserRow> {    Ok(UserRow {
        id: r.get(0)?,
        ymid: r.get(1)?,
        name: r.get(2)?,
        phone: r.get(3)?,
        email: r.get(4)?,
        avatar: r.get(5)?,
        ym_auth_token: r.get(6)?,
        is_active: r.get::<_, i64>(7)? != 0,
        last_active_ms: r.get(8)?,
    })
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS users (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    ymid                 TEXT UNIQUE NOT NULL,
    name                 TEXT NOT NULL DEFAULT '',
    phone                TEXT,
    email                TEXT,
    avatar               TEXT,
    ym_auth_token        TEXT,
    ym_auth_refresh_token TEXT,
    is_active            INTEGER NOT NULL DEFAULT 1,
    last_active_ms       INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS conversations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    title         TEXT NOT NULL DEFAULT '',
    conv_type     TEXT NOT NULL,
    topic_id      TEXT NOT NULL UNIQUE,
    created_by    INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS conversation_members (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL,
    user_id         INTEGER NOT NULL,
    role            TEXT NOT NULL DEFAULT 'member',
    UNIQUE(conversation_id, user_id)
);

CREATE TABLE IF NOT EXISTS messages (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT UNIQUE NOT NULL,
    conversation_id INTEGER NOT NULL,
    sender_id       INTEGER NOT NULL,
    msg_type        TEXT NOT NULL DEFAULT 'text',
    content         TEXT NOT NULL DEFAULT '',
    reply_to        TEXT,
    is_recalled     INTEGER NOT NULL DEFAULT 0,
    attachments     TEXT NOT NULL DEFAULT '[]',
    created_at_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msg_conv ON messages(conversation_id, seq);

CREATE TABLE IF NOT EXISTS attachments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT,
    file_name  TEXT NOT NULL,
    file_path  TEXT NOT NULL,
    file_size  INTEGER NOT NULL DEFAULT 0,
    file_type  TEXT NOT NULL DEFAULT ''
);
"#;
