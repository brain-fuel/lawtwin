// Written by `lawtwin import openapi`. Do not edit.
// ref:DEC-embed-generated-model ref:DEC-http-contract-registry
use lawtwin_petstore_model::lawspec_runtime as ls;
use lawtwin_petstore_model::lawspec_data as data;
use lawtwin_petstore_model::lawspec_abilities::lawtwin_petstore::Reference;
use crate::model::{Model as _, Outcome};
use crate::V;

pub const PACKAGE: &str = "lawtwin.petstore";
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
            "updatePet" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: data::Pet = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_updatePet(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Pet(value))
            }
            "addPet" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: data::Pet = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_addPet(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Pet(value))
            }
            "findPetsByStatus" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: data::PetStatus = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_findPetsByStatus(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::List(value.into_iter().map(|v| encode_Pet(v)).collect()))
            }
            "findPetsByTags" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::vec::Vec<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_findPetsByTags(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::List(value.into_iter().map(|v| encode_Pet(v)).collect()))
            }
            "getPetById" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_getPetById(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Pet(value))
            }
            "updatePetWithForm" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_updatePetWithForm(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Pet(value))
            }
            "deletePet" => {
                if args.len() != 2 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_deletePet(value0, value1).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "uploadFile" => {
                if args.len() != 3 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value2: std::option::Option<std::vec::Vec<u8>> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_uploadFile(value0, value1, value2).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_ApiResponse(value))
            }
            "getInventory" => {
                if args.len() != 0 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value = self.handler.try_getInventory().map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::Map(value.into_iter().map(|(k,v)| (V::Text(k), V::Int(i64::from(v)))).collect()))
            }
            "placeOrder" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::option::Option<data::Order> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_placeOrder(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Order(value))
            }
            "getOrderById" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_getOrderById(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_Order(value))
            }
            "deleteOrder" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: i64 = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_deleteOrder(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "createUser" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::option::Option<data::User> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_createUser(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_User(value))
            }
            "createUsersWithListInput" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::option::Option<std::vec::Vec<data::User>> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_createUsersWithListInput(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_User(value))
            }
            "loginUser" => {
                if args.len() != 2 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::option::Option<std::string::String> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_loginUser(value0, value1).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(V::Text(value))
            }
            "logoutUser" => {
                if args.len() != 0 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value = self.handler.try_logoutUser().map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "getUserByName" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_getUserByName(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok(encode_User(value))
            }
            "updateUser" => {
                if args.len() != 2 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value1: std::option::Option<data::User> = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_updateUser(value0, value1).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            "deleteUser" => {
                if args.len() != 1 { return Err(Outcome::Defect("wrong argument count".into())); }
                let value0: std::string::String = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;
                let value = self.handler.try_deleteUser(value0).map_err(|e| match ls::IntoValue::into_value(e) { ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit("::").next().unwrap().into()), other => Outcome::Defect(format!("unexpected failure {other:?}")) })?;
                Ok({ let _ = value; V::Unit })
            }
            other => Err(Outcome::Defect(format!("the contract has no operation {other}"))),
        })
    }
    fn state(&self) -> V { encode_State(self.handler.state()) }
    fn boxed(&self) -> Box<dyn crate::model::Model> { Box::new(self.clone()) }
}
#[allow(non_snake_case)]
fn encode_Pet(value: data::Pet) -> V {
    V::Data {ty: "Pet".into(), ctor: "Pet".into(), fields: vec![V::Maybe(value.id.map(|v| Box::new(V::Int(v)))), V::Text(value.name), V::Maybe(value.category.map(|v| Box::new(encode_Category(v)))), V::List(value.photoUrls.into_iter().map(|v| V::Text(v)).collect()), V::Maybe(value.tags.map(|v| Box::new(V::List(v.into_iter().map(|v| encode_Tag(v)).collect())))), V::Maybe(value.status.map(|v| Box::new(encode_PetStatus(v))))]}
}
#[allow(non_snake_case)]
fn encode_Category(value: data::Category) -> V {
    V::Data {ty: "Category".into(), ctor: "Category".into(), fields: vec![V::Maybe(value.id.map(|v| Box::new(V::Int(v)))), V::Maybe(value.name.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_Tag(value: data::Tag) -> V {
    V::Data {ty: "Tag".into(), ctor: "Tag".into(), fields: vec![V::Maybe(value.id.map(|v| Box::new(V::Int(v)))), V::Maybe(value.name.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_PetStatus(value: data::PetStatus) -> V {
    match value {
        data::PetStatus::Available => V::Data {ty: "PetStatus".into(), ctor: "Available".into(), fields: vec![]},
        data::PetStatus::Pending => V::Data {ty: "PetStatus".into(), ctor: "Pending".into(), fields: vec![]},
        data::PetStatus::Sold => V::Data {ty: "PetStatus".into(), ctor: "Sold".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_ApiResponse(value: data::ApiResponse) -> V {
    V::Data {ty: "ApiResponse".into(), ctor: "ApiResponse".into(), fields: vec![V::Maybe(value.code.map(|v| Box::new(V::Int(i64::from(v))))), V::Maybe(value.type_.map(|v| Box::new(V::Text(v)))), V::Maybe(value.message.map(|v| Box::new(V::Text(v))))]}
}
#[allow(non_snake_case)]
fn encode_Order(value: data::Order) -> V {
    V::Data {ty: "Order".into(), ctor: "Order".into(), fields: vec![V::Maybe(value.id.map(|v| Box::new(V::Int(v)))), V::Maybe(value.petId.map(|v| Box::new(V::Int(v)))), V::Maybe(value.quantity.map(|v| Box::new(V::Int(i64::from(v))))), V::Maybe(value.shipDate.map(|v| Box::new(V::Text(v)))), V::Maybe(value.status.map(|v| Box::new(encode_OrderStatus(v)))), V::Maybe(value.complete.map(|v| Box::new(V::Bool(v))))]}
}
#[allow(non_snake_case)]
fn encode_OrderStatus(value: data::OrderStatus) -> V {
    match value {
        data::OrderStatus::Placed => V::Data {ty: "OrderStatus".into(), ctor: "Placed".into(), fields: vec![]},
        data::OrderStatus::Approved => V::Data {ty: "OrderStatus".into(), ctor: "Approved".into(), fields: vec![]},
        data::OrderStatus::Delivered => V::Data {ty: "OrderStatus".into(), ctor: "Delivered".into(), fields: vec![]},
    }
}
#[allow(non_snake_case)]
fn encode_User(value: data::User) -> V {
    V::Data {ty: "User".into(), ctor: "User".into(), fields: vec![V::Maybe(value.id.map(|v| Box::new(V::Int(v)))), V::Maybe(value.username.map(|v| Box::new(V::Text(v)))), V::Maybe(value.firstName.map(|v| Box::new(V::Text(v)))), V::Maybe(value.lastName.map(|v| Box::new(V::Text(v)))), V::Maybe(value.email.map(|v| Box::new(V::Text(v)))), V::Maybe(value.password.map(|v| Box::new(V::Text(v)))), V::Maybe(value.phone.map(|v| Box::new(V::Text(v)))), V::Maybe(value.userStatus.map(|v| Box::new(V::Int(i64::from(v)))))]}
}
#[allow(non_snake_case)]
fn encode_State(value: data::State) -> V {
    V::Data {ty: "State".into(), ctor: "State".into(), fields: vec![
        V::Map(value.pets.into_iter().map(|(k,v)| (V::Int(k), encode_Pet(v))).collect()),
        V::Int(value.nextPetKey),
        V::Map(value.orders.into_iter().map(|(k,v)| (V::Int(k), encode_Order(v))).collect()),
        V::Int(value.nextOrderKey),
        V::Map(value.users.into_iter().map(|(k,v)| (V::Text(k), encode_User(v))).collect()),
    ]}
}
