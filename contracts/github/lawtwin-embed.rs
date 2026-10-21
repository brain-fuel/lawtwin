// Written by `lawtwin import openapi`. Do not edit.
// ref:DEC-embed-generated-model ref:DEC-http-contract-registry
use lawtwin_github_model::lawspec_runtime as ls;
use lawtwin_github_model::lawspec_data as data;
use lawtwin_github_model::lawspec_abilities::lawtwin_github::Reference;
use crate::model::{Model as _, Outcome};
use crate::V;

pub const PACKAGE: &str = "lawtwin.github";
pub const VERSION: &str = "0.1.0";
pub const WIRE_MAP: &str = include_str!("lawtwin-openapi.json");
pub const TWIN_FILE: &str = include_str!("lawtwin.json");

fn to_value(v: V) -> ls::Value {
    match v {
        V::Int(i) => ls::IntoValue::into_value(i),
        V::Float(f) => ls::Value::Float64(f),
        V::Bool(b) => ls::Value::Bool(b),
        V::Text(s) => ls::Value::Text(s),
        V::Bytes(b) => ls::Value::Bytes(b),
        V::Unit => ls::Value::Unit,
        V::Maybe(o) => ls::Value::Maybe(o.map(|v| Box::new(to_value(*v)))),
        V::List(xs) => ls::Value::List(xs.into_iter().map(to_value).collect()),
        V::Map(xs) => ls::Value::List(xs.into_iter().map(|(k,v)| ls::Value::Data(
            "lawspec.collections::type::Entry::Entry".into(), vec![to_value(k),to_value(v)]
        )).collect()),
        V::Data {ty,ctor,fields} => ls::Value::Data(
            format!("{PACKAGE}.model::type::{ty}::{ctor}"), fields.into_iter().map(to_value).collect()
        ),
    }
}

fn run<T>(body: impl FnOnce() -> Result<T, Outcome>) -> Result<T, Outcome> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|payload| Err(Outcome::Defect(crate::model::panic_text(&payload))))
}

pub struct Model { handler: Reference }
impl Clone for Model {
    fn clone(&self) -> Self {
        let handler = Reference::start();
        handler.set_state(self.handler.state());
        Self { handler }
    }
}
pub fn start() -> Box<dyn crate::model::Model> { Box::new(Model {handler: Reference::start()}) }

impl crate::model::Model for Model {
    fn call(&mut self, op: &str, args: Vec<V>) -> Result<V, Outcome> {
        let mut args = args.into_iter();
        run(|| match op {
            "issuesListLabelsForRepo" => {
                if args.len() != 4 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::option::Option<i64> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value3: std::option::Option<i64> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesListLabelsForRepo(value0, value1, value2, value3).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::List(value.into_iter().map(|v| encode_Label(v)).collect()))
            }
            "issuesCreateLabel" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: data::IssuesCreateLabelBody = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesCreateLabel(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Label(value))
            }
            "issuesGetLabel" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesGetLabel(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Label(value))
            }
            "issuesDeleteLabel" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesDeleteLabel(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "issuesUpdateLabel" => {
                if args.len() != 4 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value3: std::option::Option<data::IssuesUpdateLabelBody> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesUpdateLabel(value0, value1, value2, value3).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Label(value))
            }
            "issuesListMilestones" => {
                if args.len() != 7 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::option::Option<data::IssuesListMilestonesState> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value3: std::option::Option<data::IssuesListMilestonesSort> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value4: std::option::Option<data::IssuesListMilestonesDirection> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value5: std::option::Option<i64> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value6: std::option::Option<i64> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesListMilestones(value0, value1, value2, value3, value4, value5, value6).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::List(value.into_iter().map(|v| encode_Milestone(v)).collect()))
            }
            "issuesCreateMilestone" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: data::IssuesCreateMilestoneBody = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesCreateMilestone(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Milestone(value))
            }
            "issuesGetMilestone" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesGetMilestone(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Milestone(value))
            }
            "issuesDeleteMilestone" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesDeleteMilestone(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "issuesUpdateMilestone" => {
                if args.len() != 4 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value3: std::option::Option<data::IssuesUpdateMilestoneBody> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_issuesUpdateMilestone(value0, value1, value2, value3).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Milestone(value))
            }
            other => Err(Outcome::Defect(format!("the contract has no operation {other}"))),
        })
    }
    fn state(&self) -> V { encode_State(self.handler.state()) }
    fn boxed(&self) -> Box<dyn crate::model::Model> { Box::new(self.clone()) }
}
#[allow(non_snake_case)]
fn encode_Label(value: data::Label) -> V {
    V::Data {ty: "Label".into(), ctor: "Label".into(), fields: vec![V::Int(value.id), V::Text(value.nodeId), V::Text(value.url), V::Text(value.name), V::Maybe(value.description_.map(|v| Box::new(V::Text(v)))), V::Text(value.color), V::Bool(value.default_), V::Maybe(value.archivedAt.map(|v| Box::new(V::Text(v)))), V::Maybe(value.archivedBy.map(|v| Box::new(encode_SimpleUser(v))))]}
}
#[allow(non_snake_case)]
fn encode_SimpleUser(value: data::SimpleUser) -> V {
    V::Data {ty: "SimpleUser".into(), ctor: "SimpleUser".into(), fields: vec![V::Maybe(value.name.map(|v| Box::new(V::Text(v)))), V::Maybe(value.email.map(|v| Box::new(V::Text(v)))), V::Text(value.login), V::Int(value.id), V::Text(value.nodeId), V::Text(value.avatarUrl), V::Maybe(value.gravatarId.map(|v| Box::new(V::Text(v)))), V::Text(value.url), V::Text(value.htmlUrl), V::Text(value.followersUrl), V::Text(value.followingUrl), V::Text(value.gistsUrl), V::Text(value.starredUrl), V::Text(value.subscriptionsUrl), V::Text(value.organizationsUrl), V::Text(value.reposUrl), V::Text(value.eventsUrl), V::Text(value.receivedEventsUrl), V::Text(value.type_), V::Bool(value.siteAdmin), V::Maybe(value.starredAt.map(|v| Box::new(V::Text(v)))), V::Maybe(value.userViewType.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_IssuesCreateLabelBody(value: data::IssuesCreateLabelBody) -> V {
    V::Data {ty: "IssuesCreateLabelBody".into(), ctor: "IssuesCreateLabelBody".into(), fields: vec![V::Text(value.name), V::Maybe(value.color.map(|v| Box::new(V::Text(v)))), V::Maybe(value.description_.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_IssuesUpdateLabelBody(value: data::IssuesUpdateLabelBody) -> V {
    V::Data {ty: "IssuesUpdateLabelBody".into(), ctor: "IssuesUpdateLabelBody".into(), fields: vec![V::Maybe(value.newName.map(|v| Box::new(V::Text(v)))), V::Maybe(value.color.map(|v| Box::new(V::Text(v)))), V::Maybe(value.description_.map(|v| Box::new(V::Text(v)))), V::Maybe(value.archived.map(|v| Box::new(V::Bool(v))))]}
}
#[allow(non_snake_case)]
fn encode_IssuesListMilestonesState(value: data::IssuesListMilestonesState) -> V {
    match value {
        data::IssuesListMilestonesState::Open => V::Data {ty: "IssuesListMilestonesState".into(), ctor: "Open".into(), fields: vec![]},
        data::IssuesListMilestonesState::Closed => V::Data {ty: "IssuesListMilestonesState".into(), ctor: "Closed".into(), fields: vec![]},
        data::IssuesListMilestonesState::All => V::Data {ty: "IssuesListMilestonesState".into(), ctor: "All".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_IssuesListMilestonesSort(value: data::IssuesListMilestonesSort) -> V {
    match value {
        data::IssuesListMilestonesSort::DueOn => V::Data {ty: "IssuesListMilestonesSort".into(), ctor: "DueOn".into(), fields: vec![]},
        data::IssuesListMilestonesSort::Completeness => V::Data {ty: "IssuesListMilestonesSort".into(), ctor: "Completeness".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_IssuesListMilestonesDirection(value: data::IssuesListMilestonesDirection) -> V {
    match value {
        data::IssuesListMilestonesDirection::Asc => V::Data {ty: "IssuesListMilestonesDirection".into(), ctor: "Asc".into(), fields: vec![]},
        data::IssuesListMilestonesDirection::Desc => V::Data {ty: "IssuesListMilestonesDirection".into(), ctor: "Desc".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_Milestone(value: data::Milestone) -> V {
    V::Data {ty: "Milestone".into(), ctor: "Milestone".into(), fields: vec![V::Text(value.url), V::Text(value.htmlUrl), V::Text(value.labelsUrl), V::Int(value.id), V::Text(value.nodeId), V::Int(value.number), encode_MilestoneState(value.state_), V::Text(value.title), V::Maybe(value.description_.map(|v| Box::new(V::Text(v)))), V::Maybe(value.creator.map(|v| Box::new(encode_NullableSimpleUser(v)))), V::Int(value.openIssues), V::Int(value.closedIssues), V::Text(value.createdAt), V::Text(value.updatedAt), V::Maybe(value.closedAt.map(|v| Box::new(V::Text(v)))), V::Maybe(value.dueOn.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_MilestoneState(value: data::MilestoneState) -> V {
    match value {
        data::MilestoneState::MilestoneStateOpen => V::Data {ty: "MilestoneState".into(), ctor: "MilestoneStateOpen".into(), fields: vec![]},
        data::MilestoneState::MilestoneStateClosed => V::Data {ty: "MilestoneState".into(), ctor: "MilestoneStateClosed".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_NullableSimpleUser(value: data::NullableSimpleUser) -> V {
    V::Data {ty: "NullableSimpleUser".into(), ctor: "NullableSimpleUser".into(), fields: vec![V::Maybe(value.name.map(|v| Box::new(V::Text(v)))), V::Maybe(value.email.map(|v| Box::new(V::Text(v)))), V::Text(value.login), V::Int(value.id), V::Text(value.nodeId), V::Text(value.avatarUrl), V::Maybe(value.gravatarId.map(|v| Box::new(V::Text(v)))), V::Text(value.url), V::Text(value.htmlUrl), V::Text(value.followersUrl), V::Text(value.followingUrl), V::Text(value.gistsUrl), V::Text(value.starredUrl), V::Text(value.subscriptionsUrl), V::Text(value.organizationsUrl), V::Text(value.reposUrl), V::Text(value.eventsUrl), V::Text(value.receivedEventsUrl), V::Text(value.type_), V::Bool(value.siteAdmin), V::Maybe(value.starredAt.map(|v| Box::new(V::Text(v)))), V::Maybe(value.userViewType.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_IssuesCreateMilestoneBody(value: data::IssuesCreateMilestoneBody) -> V {
    V::Data {ty: "IssuesCreateMilestoneBody".into(), ctor: "IssuesCreateMilestoneBody".into(), fields: vec![V::Text(value.title), V::Maybe(value.state_.map(|v| Box::new(encode_MilestoneState(v)))), V::Maybe(value.description_.map(|v| Box::new(V::Text(v)))), V::Maybe(value.dueOn.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_IssuesUpdateMilestoneBody(value: data::IssuesUpdateMilestoneBody) -> V {
    V::Data {ty: "IssuesUpdateMilestoneBody".into(), ctor: "IssuesUpdateMilestoneBody".into(), fields: vec![V::Maybe(value.title.map(|v| Box::new(V::Text(v)))), V::Maybe(value.state_.map(|v| Box::new(encode_MilestoneState(v)))), V::Maybe(value.description_.map(|v| Box::new(V::Text(v)))), V::Maybe(value.dueOn.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_State(value: data::State) -> V {
    V::Data {ty: "State".into(), ctor: "State".into(), fields: vec![
        V::Map(value.labels.into_iter().map(|(k,v)| (encode_LabelKey(k), encode_Label(v))).collect()),
        V::Map(value.milestones.into_iter().map(|(k,v)| (encode_MilestoneKey(k), encode_Milestone(v))).collect()),
        V::Int(value.nextMilestoneKey),
    ]}
}
#[allow(non_snake_case)]
fn encode_LabelKey(value: data::LabelKey) -> V { V::Data {ty: "LabelKey".into(), ctor: "LabelKey".into(), fields: vec![
    V::Text(value.owner),
    V::Text(value.repo),
    V::Text(value.name)]} }
#[allow(non_snake_case)]
fn encode_MilestoneKey(value: data::MilestoneKey) -> V { V::Data {ty: "MilestoneKey".into(), ctor: "MilestoneKey".into(), fields: vec![
    V::Text(value.owner),
    V::Text(value.repo),
    V::Int(value.milestoneNumber)]} }
