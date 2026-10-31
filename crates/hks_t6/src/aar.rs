//! bo2mp lane C: the After Action Report's engine answers (BO2's lobby
//! report after a ranked match: Combat Training). BO2's engine kept the
//! last match's scoreboard and his unlocks; here the match HUD hands its
//! final scoreboard over (`set_last_match`) and the unlocks come from his
//! rank before the match (STATS_LOCATION_STABLE) and now.
//!
//! The nemesis (name, rank, kills each way) is the scripts' own
//! `AfterActionReportStats`, read from his stats as BO2 reads it.
//!
//! The match's medals (`GetRecentMedals`) are the ones his HUD was told
//! of (`medal_received`, BO2's _medals::codecallback_medal), counted by
//! their mp/scoreInfo.csv row.
//!
//! Not recorded yet (answered empty): the match's challenges
//! (`GetRecentChallenges`), recently unlocked attachments
//! (`GetNumBulletWeapons` stays unanswered, so the report skips them, as it
//! does when there are none).

use std::sync::Mutex;

use crate::host::Host;
use crate::mp::{BoardRow, Mp};
use crate::value::{LuaTable, Value};

/// The last match's final scoreboard: its game type, the scoreboard's
/// columns (`score`, `kills`, ...) and every player's row. His own row is
/// client 0 (the listen server's local player).
#[derive(Clone, Debug, Default)]
pub struct LastMatch {
    pub gametype: String,
    pub columns: Vec<String>,
    pub rows: Vec<BoardRow>,
}

static LAST_MATCH: Mutex<Option<LastMatch>> = Mutex::new(None);

/// The match HUD's scoreboard as it changes (the last one stands).
pub fn set_last_match(m: LastMatch) {
    if let Ok(mut l) = LAST_MATCH.lock() {
        *l = Some(m);
    }
}

fn last_match() -> LastMatch {
    LAST_MATCH.lock().ok().and_then(|l| l.clone()).unwrap_or_default()
}

/// The match's medals: each scoreInfo row and how many times he got it.
static MEDALS: Mutex<Vec<(i32, u32)>> = Mutex::new(Vec::new());

/// A new match: no medals yet.
pub fn new_match() {
    if let Ok(mut m) = MEDALS.lock() {
        m.clear();
    }
}

/// He got the medal on this scoreInfo row.
pub fn record_medal(row: i32) {
    if let Ok(mut m) = MEDALS.lock() {
        match m.iter_mut().find(|(r, _)| *r == row) {
            Some((_, n)) => *n += 1,
            None => m.push((row, 1)),
        }
    }
}

fn medals() -> Vec<(i32, u32)> {
    MEDALS.lock().map(|m| m.clone()).unwrap_or_default()
}

/// GetRecentMedals' answer: `{{row, value}}`, a medal's row and its count
/// (the report shows "x<count>" past one, and sorts them itself).
fn medals_table(medals: &[(i32, u32)]) -> Value {
    let t = LuaTable::new_ref();
    for (n, (row, count)) in medals.iter().enumerate() {
        let e = LuaTable::new_ref();
        e.borrow_mut().set_str("row", Value::Num(*row as f32));
        e.borrow_mut().set_str("value", Value::Num(*count as f32));
        t.borrow_mut().store(Value::Num((n + 1) as f32), Value::Table(e));
    }
    Value::Table(t)
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Nil)
}

/// A column's header key: "MPUI_" and the column, upper case (the report
/// adds "_CAPS": MPUI_KILLS_CAPS).
fn header(column: &str) -> String {
    format!("MPUI_{}", column.to_ascii_uppercase())
}

pub fn install(host: &mut Host, mp: &Mp) {
    // GetAARScoreboard(controller): {gametype, xpScale, members = {{name,
    // isSelf, team, clientNum, scoreBoardColumn1..N}}}.
    host.bind("Engine", "GetAARScoreboard", |_, _| {
        let last = last_match();
        let t = LuaTable::new_ref();
        let members = LuaTable::new_ref();
        for (n, row) in last.rows.iter().enumerate() {
            let m = LuaTable::new_ref();
            {
                let mut m = m.borrow_mut();
                m.set_str("name", Value::str(&row.name));
                m.set_str("isSelf", Value::Boolean(row.client == 0));
                m.set_str("team", Value::Num(row.team as f32));
                m.set_str("clientNum", Value::Num(row.client as f32));
                for (i, v) in row.cols.iter().enumerate() {
                    m.set_str(&format!("scoreBoardColumn{}", i + 1), Value::Num(*v));
                }
            }
            members.borrow_mut().store(Value::Num((n + 1) as f32), Value::Table(m));
        }
        {
            let mut t = t.borrow_mut();
            t.set_str("gametype", Value::str(if last.gametype.is_empty() { "tdm" } else { &last.gametype }));
            t.set_str("xpScale", Value::Num(1.0));
            t.set_str("members", Value::Table(members));
        }
        Ok(vec![Value::Table(t)])
    });
    // GetScoreboardColumnHeader(controller, column from 0).
    host.bind("UIExpression", "GetScoreboardColumnHeader", |_, a| {
        let i = arg(&a, 1).as_num().map_or(0, |n| n.max(0.0) as usize);
        Ok(vec![Value::str(&last_match().columns.get(i).map(|c| header(c)).unwrap_or_default())])
    });
    // GetRecentMedals(controller, the game type's XP column): his medals of
    // the match (each one he was shown carried XP in it).
    host.bind("Engine", "GetRecentMedals", |_, _| Ok(vec![medals_table(&medals())]));
    host.bind("Engine", "GetRecentChallenges", |_, _| Ok(vec![Value::Table(LuaTable::new_ref())]));
    host.bind("UIExpression", "IsSuperUser", |_, _| Ok(vec![Value::Num(0.0)]));
    // GetRecentlyUnlockedItems(controller): the items his new ranks
    // unlocked ({itemIndex, itemImage, itemName, itemDesc}: the cards the
    // report shows); GetNumFeatureUnlocks: how many of them are features
    // (Create-a-Class, scorestreaks...).
    let m = mp.clone();
    host.bind("Engine", "GetRecentlyUnlockedItems", move |_, _| {
        let t = LuaTable::new_ref();
        for (n, (index, image, name, desc, _)) in m.borrow().recent_unlocks().into_iter().enumerate() {
            let e = LuaTable::new_ref();
            e.borrow_mut().set_str("itemIndex", Value::Num(index as f32));
            e.borrow_mut().set_str("itemImage", Value::str(&image));
            e.borrow_mut().set_str("itemName", Value::str(&name));
            e.borrow_mut().set_str("itemDesc", Value::str(&desc));
            t.borrow_mut().store(Value::Num((n + 1) as f32), Value::Table(e));
        }
        Ok(vec![Value::Table(t)])
    });
    // GetRecentUnlocks(controller): the unlock tokens his new ranks gave
    // (his token pool now less before the match).
    for name in ["GetRecentUnlocks", "GetRecentUnlockTokens"] {
        let m = mp.clone();
        host.bind("UIExpression", name, move |_, _| {
            let m = m.borrow();
            let path = bo2_profile::UNLOCK_TOKENS;
            let before = m.stable_stat(path).as_num().unwrap_or(0.0);
            let now = m.stat(path).as_num().unwrap_or(0.0);
            Ok(vec![Value::Num((now - before).max(0.0))])
        });
    }
    let m = mp.clone();
    host.bind("UIExpression", "GetNumFeatureUnlocks", move |_, _| {
        let n = m.borrow().recent_unlocks().iter().filter(|u| u.4).count();
        Ok(vec![Value::Num(n as f32)])
    });
    // GetUnlockedFeatureItemIndex(controller, n): the n-th (from 0) of
    // those that are features; GetItemGroupByIndex(controller, index):
    // an item's group (the card's "MPUI_<group>" heading).
    let m = mp.clone();
    host.bind("UIExpression", "GetUnlockedFeatureItemIndex", move |_, a| {
        let n = arg(&a, 1).as_num().map_or(0, |n| n.max(0.0) as usize);
        let hit = m.borrow().recent_unlocks().into_iter().filter(|u| u.4).nth(n);
        Ok(vec![hit.map_or(Value::Nil, |u| Value::Num(u.0 as f32))])
    });
    let m = mp.clone();
    host.bind("UIExpression", "GetItemGroupByIndex", move |_, a| {
        let i = arg(&a, 1).as_num().map_or(0, |n| n as i32);
        Ok(vec![m.borrow().profile.item(i).map_or(Value::Nil, |it| Value::str(&it.group.to_ascii_uppercase()))])
    });
    // String tables: TableLookupColumnNumForValue(table, row, value) - the
    // column holding value in that row; TableGetColumnValueForRow(table,
    // row, column).
    let v = host.values.clone();
    host.bind("Engine", "TableLookupColumnNumForValue", move |_, a| {
        let vb = v.borrow();
        let rows = crate::mp::table_rows(&vb, &arg(&a, 0).to_string());
        let row = arg(&a, 1).as_num().map_or(0, |n| n.max(0.0) as usize);
        let want = arg(&a, 2).to_string();
        let col = rows.get(row).and_then(|r| r.iter().position(|c| c.trim().eq_ignore_ascii_case(&want)));
        Ok(vec![Value::Num(col.map_or(-1.0, |c| c as f32))])
    });
    let v = host.values.clone();
    host.bind("Engine", "TableGetColumnValueForRow", move |_, a| {
        let vb = v.borrow();
        let rows = crate::mp::table_rows(&vb, &arg(&a, 0).to_string());
        let row = arg(&a, 1).as_num().map_or(0, |n| n.max(0.0) as usize);
        let col = arg(&a, 2).as_num().map_or(0, |n| n.max(0.0) as usize);
        Ok(vec![Value::str(rows.get(row).and_then(|r| r.get(col)).map_or("", String::as_str))])
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medals_count_per_row_and_start_over() {
        new_match();
        record_medal(12);
        record_medal(40);
        record_medal(12);
        let Value::Table(t) = medals_table(&medals()) else { panic!("not a table") };
        let entry = |n: f32| match t.borrow().read_key(&Value::Num(n)) {
            Value::Table(e) => {
                let e = e.borrow();
                (e.get_str("row").as_num(), e.get_str("value").as_num())
            }
            _ => (None, None),
        };
        assert_eq!(entry(1.0), (Some(12.0), Some(2.0)));
        assert_eq!(entry(2.0), (Some(40.0), Some(1.0)));
        assert_eq!(entry(3.0), (None, None));
        new_match();
        assert!(medals().is_empty());
    }
}
