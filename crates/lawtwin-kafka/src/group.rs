//! The classic group protocol over the contract's coordinator.
//!
//! The contract's coordinator assigns partitions itself (as in KIP-848): a
//! member joins with its topics, and a sync gives it its partitions. Real
//! clients still speak the classic protocol, where every member must rejoin
//! before a new generation starts and a leader computes the assignment. The
//! twin keeps the classic protocol's rounds here and runs the contract's
//! operations when a round completes: a member that did not rejoin leaves
//! (`leaveGroupRequest`), each member that did joins (`joinGroupRequest`),
//! and a sync answers with the contract's assignment, whatever the leader
//! proposed. So the generations and assignments clients see are the
//! model's.

use crate::faults::Fault;
use crate::history::{Op, Outcome, Source};
use crate::model::{Failure, data::KafkaError};
use crate::twin::{Core, Twin};
use bytes::Bytes;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::oneshot;

/// Kafka error codes the coordinator answers.
pub mod code {
    /// No error.
    pub const NONE: i16 = 0;
    /// ILLEGAL_GENERATION.
    pub const ILLEGAL_GENERATION: i16 = 22;
    /// INVALID_GROUP_ID.
    pub const INVALID_GROUP_ID: i16 = 24;
    /// UNKNOWN_MEMBER_ID.
    pub const UNKNOWN_MEMBER_ID: i16 = 25;
    /// REBALANCE_IN_PROGRESS.
    pub const REBALANCE_IN_PROGRESS: i16 = 27;
    /// INCONSISTENT_GROUP_PROTOCOL.
    pub const INCONSISTENT_GROUP_PROTOCOL: i16 = 23;
    /// MEMBER_ID_REQUIRED.
    pub const MEMBER_ID_REQUIRED: i16 = 79;
    /// UNKNOWN_SERVER_ERROR.
    pub const UNKNOWN_SERVER_ERROR: i16 = -1;
}

/// A member as the classic protocol sees it.
#[derive(Clone, Debug)]
pub struct MemberWire {
    /// The client's id.
    pub client_id: String,
    /// The client's host.
    pub client_host: String,
    /// Its static instance id, if any.
    pub instance_id: Option<String>,
    /// The protocols it offers (assignor names) with their metadata.
    pub protocols: Vec<(String, Bytes)>,
    /// The topics it subscribes to, sorted.
    pub topics: Vec<String>,
    /// Its session timeout, in milliseconds.
    pub session_timeout_ms: i64,
    /// When it was last heard from, on the twin clock.
    pub last_seen_ms: i64,
    /// Whether the model counts it as a member.
    pub in_model: bool,
}

/// What a JoinGroup answers.
#[derive(Clone, Debug, Default)]
pub struct JoinReply {
    /// The error code.
    pub error: i16,
    /// The generation.
    pub generation: i32,
    /// The chosen protocol (assignor).
    pub protocol_name: Option<String>,
    /// The protocol type, such as `consumer`.
    pub protocol_type: Option<String>,
    /// The leader's member id.
    pub leader: String,
    /// This member's id.
    pub member_id: String,
    /// For the leader, every member with its instance id and metadata.
    pub members: Vec<(String, Option<String>, Bytes)>,
}

/// A rebalance in progress: the members that have rejoined so far.
#[derive(Debug, Default)]
pub struct Round {
    /// Its number in the group, to tell rounds apart.
    pub id: u64,
    /// The members that have rejoined.
    pub joined: BTreeSet<String>,
    /// The members waiting for the round to complete.
    pub waiters: Vec<(String, oneshot::Sender<JoinReply>)>,
    /// Whether a rebalance storm started it.
    pub forced: bool,
}

/// A group, as the classic protocol sees it.
#[derive(Debug, Default)]
pub struct GroupWire {
    /// Its members.
    pub members: BTreeMap<String, MemberWire>,
    /// Member ids handed out with MEMBER_ID_REQUIRED, not yet used.
    pub pending_ids: BTreeSet<String>,
    /// The rebalance in progress.
    pub round: Option<Round>,
    /// The protocol type of its members.
    pub protocol_type: Option<String>,
    /// The protocol chosen in the last round.
    pub protocol_name: Option<String>,
    /// The leader chosen in the last round.
    pub leader: Option<String>,
    rounds: u64,
}

/// A request to join a group, from the wire.
#[derive(Clone, Debug)]
pub struct JoinRequest {
    /// The group.
    pub group: String,
    /// The member id the client gave; empty for a new member.
    pub member_id: String,
    /// The static instance id.
    pub instance_id: Option<String>,
    /// The client's id.
    pub client_id: String,
    /// The client's host.
    pub client_host: String,
    /// The protocol type.
    pub protocol_type: String,
    /// The protocols offered, with their metadata.
    pub protocols: Vec<(String, Bytes)>,
    /// The session timeout.
    pub session_timeout_ms: i32,
    /// Whether the client understands MEMBER_ID_REQUIRED (JoinGroup v4 and later).
    pub requires_member_id: bool,
}

/// The topics a consumer's subscription metadata names, sorted.
pub fn subscribed_topics(protocol_type: &str, metadata: &Bytes) -> Vec<String> {
    use kafka_protocol::messages::ConsumerProtocolSubscription;
    use kafka_protocol::protocol::Decodable;
    if protocol_type != "consumer" || metadata.len() < 2 {
        return vec![];
    }
    let mut buf = metadata.clone();
    let version = i16::from_be_bytes([buf[0], buf[1]]).clamp(0, 3);
    let _ = bytes::Buf::get_i16(&mut buf);
    match ConsumerProtocolSubscription::decode(&mut buf, version) {
        Ok(s) => {
            let set: BTreeSet<String> = s.topics.iter().map(|t| t.to_string()).collect();
            set.into_iter().collect()
        }
        Err(_) => vec![],
    }
}

/// The classic protocol's error code for a refusal of the contract.
pub fn group_error(f: &Failure) -> i16 {
    match f {
        Failure::Kafka(KafkaError::IllegalGeneration) => code::ILLEGAL_GENERATION,
        Failure::Kafka(KafkaError::UnknownMemberId) => code::UNKNOWN_MEMBER_ID,
        Failure::Kafka(KafkaError::InvalidGroupId) => code::INVALID_GROUP_ID,
        Failure::Kafka(KafkaError::UnknownTopicOrPartition) => 3,
        _ => code::UNKNOWN_SERVER_ERROR,
    }
}

fn storm_wanted(core: &Core, group: &str) -> bool {
    core.faults.iter().any(|a| {
        matches!(&a.fault, Fault::RebalanceStorm { rounds, group: only } if *rounds > 0 && only.as_deref().is_none_or(|g| g == group))
    })
}

fn storm_used(core: &mut Core, group: &str) {
    for a in core.faults.iter_mut() {
        if let Fault::RebalanceStorm { rounds, group: only } = &mut a.fault {
            if *rounds > 0 && only.as_deref().is_none_or(|g| g == group) {
                *rounds -= 1;
                break;
            }
        }
    }
    core.faults.retain(|a| !matches!(a.fault, Fault::RebalanceStorm { rounds: 0, .. }));
}

/// Starts a round in a group, and a timer that completes it when members
/// fail to rejoin in time.
fn start_round(twin: &Arc<Twin>, gw: &mut GroupWire, group: &str, forced: bool) {
    gw.rounds += 1;
    let id = gw.rounds;
    gw.round = Some(Round { id, forced, ..Default::default() });
    let twin = twin.clone();
    let group = group.to_string();
    let wait = std::time::Duration::from_millis(twin.config.rebalance_wait_ms);
    tokio::spawn(async move {
        tokio::time::sleep(wait).await;
        let mut core = twin.lock();
        let mut gw = core.groups.remove(&group).unwrap_or_default();
        if gw.round.as_ref().is_some_and(|r| r.id == id) {
            complete_round(&mut core, &mut gw, &group);
        }
        core.groups.insert(group, gw);
    });
}

fn round_complete(gw: &GroupWire) -> bool {
    match &gw.round {
        None => false,
        Some(r) => gw.members.iter().all(|(id, m)| !m.in_model || r.joined.contains(id)) && !r.joined.is_empty(),
    }
}

/// Completes a round: members that did not rejoin leave, those that did
/// join, and every waiting member gets the new generation.
pub fn complete_round(core: &mut Core, gw: &mut GroupWire, group: &str) {
    let Some(round) = gw.round.take() else { return };
    let gone: Vec<String> =
        gw.members.iter().filter(|(id, m)| m.in_model && !round.joined.contains(*id)).map(|(id, _)| id.clone()).collect();
    for id in gone {
        let _ = core.call(Source::Coordinator, Op::LeaveGroupRequest { group: group.into(), member: id.clone() });
        gw.members.remove(&id);
        core.control(Source::Coordinator, "member removed", json!({ "group": group, "member": id, "why": "it did not rejoin in time" }));
    }
    let mut refused: BTreeMap<String, i16> = BTreeMap::new();
    for id in &round.joined {
        let Some(m) = gw.members.get(id) else { continue };
        let topics = m.topics.clone();
        match core.call(Source::Wire, Op::JoinGroupRequest { group: group.into(), member: id.clone(), topics }) {
            Ok(_) => {
                if let Some(m) = gw.members.get_mut(id) {
                    m.in_model = true;
                }
            }
            Err(f) => {
                refused.insert(id.clone(), group_error(&f));
                gw.members.remove(id);
            }
        }
    }
    let joined: Vec<String> = round.joined.iter().filter(|id| gw.members.contains_key(*id)).cloned().collect();
    if round.forced {
        if let Some(leader) = joined.first() {
            let topics = gw.members[leader].topics.clone();
            let _ = core.call(Source::Fault, Op::LeaveGroupRequest { group: group.into(), member: leader.clone() });
            let _ = core.call(Source::Fault, Op::JoinGroupRequest { group: group.into(), member: leader.clone(), topics });
        }
        storm_used(core, group);
    }
    let generation = match core.peek(&Op::DescribeGroupRequest { group: group.into() }) {
        Ok(Outcome::Group { generation, .. }) => generation,
        _ => -1,
    };
    let leader = joined.first().cloned().unwrap_or_default();
    // The protocol: the leader's first that every member offers.
    let protocol = gw.members.get(&leader).and_then(|l| {
        l.protocols
            .iter()
            .map(|(n, _)| n.clone())
            .find(|n| {
                joined
                    .iter()
                    .filter(|id| !gw.members[*id].protocols.is_empty())
                    .all(|id| gw.members[id].protocols.iter().any(|(p, _)| p == n))
            })
    });
    gw.leader = Some(leader.clone());
    gw.protocol_name = protocol.clone();
    let members: Vec<(String, Option<String>, Bytes)> = joined
        .iter()
        .map(|id| {
            let m = &gw.members[id];
            let meta = protocol
                .as_ref()
                .and_then(|p| m.protocols.iter().find(|(n, _)| n == p).map(|(_, b)| b.clone()))
                .unwrap_or_default();
            (id.clone(), m.instance_id.clone(), meta)
        })
        .collect();
    for (id, tx) in round.waiters {
        let reply = match refused.get(&id) {
            Some(code) => JoinReply { error: *code, member_id: id.clone(), generation: -1, ..Default::default() },
            None if !gw.members.contains_key(&id) => {
                JoinReply { error: code::UNKNOWN_MEMBER_ID, member_id: id.clone(), generation: -1, ..Default::default() }
            }
            None if protocol.is_none() => {
                JoinReply { error: code::INCONSISTENT_GROUP_PROTOCOL, member_id: id.clone(), generation: -1, ..Default::default() }
            }
            None => JoinReply {
                error: code::NONE,
                generation,
                protocol_name: protocol.clone(),
                protocol_type: gw.protocol_type.clone(),
                leader: leader.clone(),
                member_id: id.clone(),
                members: if id == leader { members.clone() } else { vec![] },
            },
        };
        let _ = tx.send(reply);
    }
}

/// JoinGroup.
pub async fn join(twin: &Arc<Twin>, req: JoinRequest) -> JoinReply {
    let rx = {
        let mut core = twin.lock();
        let now = core.clock.now_ms();
        let mut gw = core.groups.remove(&req.group).unwrap_or_default();
        let reply = join_locked(twin, &mut core, &mut gw, &req, now);
        core.groups.insert(req.group.clone(), gw);
        match reply {
            Ok(reply) => return reply,
            Err(rx) => rx,
        }
    };
    rx.await.unwrap_or(JoinReply { error: code::UNKNOWN_SERVER_ERROR, generation: -1, ..Default::default() })
}

fn join_locked(
    twin: &Arc<Twin>,
    core: &mut Core,
    gw: &mut GroupWire,
    req: &JoinRequest,
    now: i64,
) -> Result<JoinReply, oneshot::Receiver<JoinReply>> {
    if req.group.is_empty() {
        return Ok(JoinReply { error: code::INVALID_GROUP_ID, generation: -1, ..Default::default() });
    }
    let mut member_id = req.member_id.clone();
    if member_id.is_empty() {
        member_id = core.member_id(&req.client_id);
        if req.requires_member_id {
            gw.pending_ids.insert(member_id.clone());
            return Ok(JoinReply { error: code::MEMBER_ID_REQUIRED, member_id, generation: -1, ..Default::default() });
        }
    } else if !gw.members.contains_key(&member_id) && !gw.pending_ids.remove(&member_id) {
        return Ok(JoinReply { error: code::UNKNOWN_MEMBER_ID, member_id, generation: -1, ..Default::default() });
    }
    if gw.protocol_type.as_ref().is_some_and(|t| t != &req.protocol_type) && gw.members.values().any(|m| m.in_model) {
        return Ok(JoinReply { error: code::INCONSISTENT_GROUP_PROTOCOL, member_id, generation: -1, ..Default::default() });
    }
    gw.protocol_type = Some(req.protocol_type.clone());
    let topics = req.protocols.first().map(|(_, meta)| subscribed_topics(&req.protocol_type, meta)).unwrap_or_default();
    let previous = gw.members.get(&member_id);
    let changed = previous.is_none_or(|m| !m.in_model || m.topics != topics);
    let in_model = previous.is_some_and(|m| m.in_model);
    gw.members.insert(
        member_id.clone(),
        MemberWire {
            client_id: req.client_id.clone(),
            client_host: req.client_host.clone(),
            instance_id: req.instance_id.clone(),
            protocols: req.protocols.clone(),
            topics: topics.clone(),
            session_timeout_ms: req.session_timeout_ms.max(1) as i64,
            last_seen_ms: now,
            in_model,
        },
    );
    if gw.round.is_none() && !changed && !storm_wanted(core, &req.group) {
        // An unchanged member rejoining a stable group learns the current
        // generation, as the contract's joinGroup says.
        let result = core.call(Source::Wire, Op::JoinGroupRequest { group: req.group.clone(), member: member_id.clone(), topics });
        return Ok(match result {
            Ok(Outcome::Number(generation)) => {
                let leader = gw.leader.clone().filter(|l| gw.members.contains_key(l)).unwrap_or_else(|| member_id.clone());
                let protocol = gw.protocol_name.clone().or_else(|| req.protocols.first().map(|(n, _)| n.clone()));
                let members = if leader == member_id {
                    gw.members
                        .iter()
                        .filter(|(_, m)| m.in_model)
                        .map(|(id, m)| {
                            let meta = m
                                .protocols
                                .iter()
                                .find(|(n, _)| Some(n) == protocol.as_ref())
                                .map(|(_, b)| b.clone())
                                .unwrap_or_default();
                            (id.clone(), m.instance_id.clone(), meta)
                        })
                        .collect()
                } else {
                    vec![]
                };
                JoinReply {
                    error: code::NONE,
                    generation: generation as i32,
                    protocol_name: protocol,
                    protocol_type: gw.protocol_type.clone(),
                    leader,
                    member_id,
                    members,
                }
            }
            Ok(_) => JoinReply { error: code::UNKNOWN_SERVER_ERROR, member_id, generation: -1, ..Default::default() },
            Err(f) => JoinReply { error: group_error(&f), member_id, generation: -1, ..Default::default() },
        });
    }
    if gw.round.is_none() {
        let forced = !changed && storm_wanted(core, &req.group);
        start_round(twin, gw, &req.group, forced);
    }
    let (tx, rx) = oneshot::channel();
    let round = gw.round.as_mut().expect("a round");
    round.joined.insert(member_id.clone());
    round.waiters.retain(|(id, _)| id != &member_id);
    round.waiters.push((member_id, tx));
    if round_complete(gw) {
        complete_round(core, gw, &req.group);
    }
    Err(rx)
}

/// Heartbeat: the error code.
pub fn heartbeat(twin: &Arc<Twin>, group: &str, member: &str, generation: i32) -> i16 {
    let mut core = twin.lock();
    let now = core.clock.now_ms();
    let mut gw = core.groups.remove(group).unwrap_or_default();
    if let Some(m) = gw.members.get_mut(member) {
        m.last_seen_ms = now;
    }
    let result = core.call(Source::Wire, Op::HeartbeatRequest { group: group.into(), member: member.into(), generation });
    let in_model = gw.members.get(member).is_some_and(|m| m.in_model);
    if result.is_ok() && in_model && gw.round.is_none() && storm_wanted(&core, group) {
        start_round(twin, &mut gw, group, true);
    }
    let rejoining = gw.round.as_ref().is_some_and(|r| !r.joined.contains(member));
    let code = match &result {
        Ok(_) if rejoining => code::REBALANCE_IN_PROGRESS,
        Ok(_) => code::NONE,
        Err(Failure::Kafka(KafkaError::IllegalGeneration)) if in_model => code::REBALANCE_IN_PROGRESS,
        Err(f) => group_error(f),
    };
    core.groups.insert(group.into(), gw);
    code
}

/// SyncGroup: the error code and the member's partitions.
pub fn sync(twin: &Arc<Twin>, group: &str, member: &str, generation: i32) -> (i16, Vec<(String, i32)>) {
    let mut core = twin.lock();
    let now = core.clock.now_ms();
    let in_round = core.groups.get(group).is_some_and(|g| g.round.is_some());
    if let Some(m) = core.groups.get_mut(group).and_then(|g| g.members.get_mut(member)) {
        m.last_seen_ms = now;
    }
    if in_round {
        return (code::REBALANCE_IN_PROGRESS, vec![]);
    }
    match core.call(Source::Wire, Op::SyncGroupRequest { group: group.into(), member: member.into(), generation }) {
        Ok(Outcome::Partitions(tps)) => (code::NONE, tps.into_iter().map(|t| (t.topic, t.partition)).collect()),
        Ok(_) => (code::UNKNOWN_SERVER_ERROR, vec![]),
        Err(f) => (group_error(&f), vec![]),
    }
}

/// LeaveGroup: the error code for each member.
pub fn leave(twin: &Arc<Twin>, group: &str, members: &[String]) -> Vec<i16> {
    let mut core = twin.lock();
    let mut gw = core.groups.remove(group).unwrap_or_default();
    let mut codes = vec![];
    for member in members {
        let known = gw.members.remove(member);
        if let Some(round) = gw.round.as_mut() {
            round.joined.remove(member);
            let mut kept = vec![];
            for (id, tx) in round.waiters.drain(..) {
                if &id == member {
                    let _ = tx.send(JoinReply { error: code::UNKNOWN_MEMBER_ID, member_id: id, generation: -1, ..Default::default() });
                } else {
                    kept.push((id, tx));
                }
            }
            round.waiters = kept;
        }
        let code = match known {
            Some(m) if m.in_model => {
                match core.call(Source::Wire, Op::LeaveGroupRequest { group: group.into(), member: member.clone() }) {
                    Ok(_) => code::NONE,
                    Err(f) => group_error(&f),
                }
            }
            Some(_) => code::NONE,
            None => code::UNKNOWN_MEMBER_ID,
        };
        codes.push(code);
    }
    if round_complete(&gw) {
        complete_round(&mut core, &mut gw, group);
    } else if gw.round.as_ref().is_some_and(|r| r.joined.is_empty() && r.waiters.is_empty())
        && !gw.members.values().any(|m| m.in_model)
    {
        gw.round = None;
    }
    core.groups.insert(group.into(), gw);
    codes
}

/// Removes members whose session timed out on the twin clock: each leaves
/// the group (`leaveGroupRequest`, source coordinator).
pub fn expire_sessions(core: &mut Core) {
    let now = core.clock.now_ms();
    let groups: Vec<String> = core.groups.keys().cloned().collect();
    for group in groups {
        let mut gw = core.groups.remove(&group).unwrap_or_default();
        let joined: BTreeSet<String> = gw.round.as_ref().map(|r| r.joined.clone()).unwrap_or_default();
        let expired: Vec<String> = gw
            .members
            .iter()
            .filter(|(id, m)| m.in_model && !joined.contains(*id) && now - m.last_seen_ms > m.session_timeout_ms)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            gw.members.remove(&id);
            let _ = core.call(Source::Coordinator, Op::LeaveGroupRequest { group: group.clone(), member: id.clone() });
            core.control(Source::Coordinator, "member removed", json!({ "group": group, "member": id, "why": "its session timed out" }));
        }
        if round_complete(&gw) {
            complete_round(core, &mut gw, &group);
        }
        core.groups.insert(group, gw);
    }
}
