// Written by `lawtwin import openapi`, and written again on every import.
//! The native handlers of lawtwin.petstore, for running its laws: each answers
//! through the reference model, held by the case's Service. A connection
//! to the HTTP twin or to the real service takes its place in conformance
//! runs.
#![allow(unused_variables, unused_imports, non_snake_case)]
use crate::lawspec_abilities::lawtwin_petstore as abilities;
use crate::lawspec_runtime as ls;
use std::sync::Arc;

/// What a Service holds: the reference model.
pub struct ReferenceService {
    model: Arc<abilities::Reference>,
}

fn reference(service: &crate::lawspec_data::Service) -> &ReferenceService {
    service.native::<ReferenceService>().unwrap_or_else(|error| panic!("{error}"))
}

// LawSpec: (Unit -> lawtwin.petstore::type::Service)
pub fn connectService(value0: ()) -> crate::lawspec_data::Service {
    ls::Handle::new(ReferenceService { model: Arc::new(abilities::Reference::new(&ls::Context::default())) })
}

// LawSpec: (lawtwin.petstore::type::Service -> Unit)
pub fn disconnectService(value0: crate::lawspec_data::Service) -> () {}

/// PetApi, answered by the reference model.
pub struct PetApiHandler {
    pub resource: crate::lawspec_data::Service,
}

impl PetApiHandler {
    pub fn new(resource: crate::lawspec_data::Service) -> Self {
        Self { resource }
    }
}

impl abilities::PetApi for PetApiHandler {
    fn updatePet(&self, value0: crate::lawspec_data::Pet) -> crate::lawspec_data::Pet {
        <abilities::Reference as abilities::PetApi>::updatePet(&reference(&self.resource).model, value0)
    }

    fn addPet(&self, value0: crate::lawspec_data::Pet) -> crate::lawspec_data::Pet {
        <abilities::Reference as abilities::PetApi>::addPet(&reference(&self.resource).model, value0)
    }

    fn findPetsByStatus(&self, value0: crate::lawspec_data::PetStatus) -> std::vec::Vec<crate::lawspec_data::Pet> {
        <abilities::Reference as abilities::PetApi>::findPetsByStatus(&reference(&self.resource).model, value0)
    }

    fn findPetsByTags(&self, value0: std::vec::Vec<std::string::String>) -> std::vec::Vec<crate::lawspec_data::Pet> {
        <abilities::Reference as abilities::PetApi>::findPetsByTags(&reference(&self.resource).model, value0)
    }

    fn getPetById(&self, value0: i64) -> crate::lawspec_data::Pet {
        <abilities::Reference as abilities::PetApi>::getPetById(&reference(&self.resource).model, value0)
    }

    fn updatePetWithForm(&self, value0: i64, value1: std::option::Option<std::string::String>, value2: std::option::Option<std::string::String>) -> crate::lawspec_data::Pet {
        <abilities::Reference as abilities::PetApi>::updatePetWithForm(&reference(&self.resource).model, value0, value1, value2)
    }

    fn deletePet(&self, value0: i64, value1: std::option::Option<std::string::String>) -> () {
        <abilities::Reference as abilities::PetApi>::deletePet(&reference(&self.resource).model, value0, value1)
    }

    fn uploadFile(&self, value0: i64, value1: std::option::Option<std::string::String>, value2: std::option::Option<std::vec::Vec<u8>>) -> crate::lawspec_data::ApiResponse {
        <abilities::Reference as abilities::PetApi>::uploadFile(&reference(&self.resource).model, value0, value1, value2)
    }
}

/// StoreApi, answered by the reference model.
pub struct StoreApiHandler {
    pub resource: crate::lawspec_data::Service,
}

impl StoreApiHandler {
    pub fn new(resource: crate::lawspec_data::Service) -> Self {
        Self { resource }
    }
}

impl abilities::StoreApi for StoreApiHandler {
    fn getInventory(&self) -> std::collections::BTreeMap<std::string::String, i32> {
        <abilities::Reference as abilities::StoreApi>::getInventory(&reference(&self.resource).model)
    }

    fn placeOrder(&self, value0: std::option::Option<crate::lawspec_data::Order>) -> crate::lawspec_data::Order {
        <abilities::Reference as abilities::StoreApi>::placeOrder(&reference(&self.resource).model, value0)
    }

    fn getOrderById(&self, value0: i64) -> crate::lawspec_data::Order {
        <abilities::Reference as abilities::StoreApi>::getOrderById(&reference(&self.resource).model, value0)
    }

    fn deleteOrder(&self, value0: i64) -> () {
        <abilities::Reference as abilities::StoreApi>::deleteOrder(&reference(&self.resource).model, value0)
    }
}

/// UserApi, answered by the reference model.
pub struct UserApiHandler {
    pub resource: crate::lawspec_data::Service,
}

impl UserApiHandler {
    pub fn new(resource: crate::lawspec_data::Service) -> Self {
        Self { resource }
    }
}

impl abilities::UserApi for UserApiHandler {
    fn createUser(&self, value0: std::option::Option<crate::lawspec_data::User>) -> crate::lawspec_data::User {
        <abilities::Reference as abilities::UserApi>::createUser(&reference(&self.resource).model, value0)
    }

    fn createUsersWithListInput(&self, value0: std::option::Option<std::vec::Vec<crate::lawspec_data::User>>) -> crate::lawspec_data::User {
        <abilities::Reference as abilities::UserApi>::createUsersWithListInput(&reference(&self.resource).model, value0)
    }

    fn loginUser(&self, value0: std::option::Option<std::string::String>, value1: std::option::Option<std::string::String>) -> std::string::String {
        <abilities::Reference as abilities::UserApi>::loginUser(&reference(&self.resource).model, value0, value1)
    }

    fn logoutUser(&self) -> () {
        <abilities::Reference as abilities::UserApi>::logoutUser(&reference(&self.resource).model)
    }

    fn getUserByName(&self, value0: std::string::String) -> crate::lawspec_data::User {
        <abilities::Reference as abilities::UserApi>::getUserByName(&reference(&self.resource).model, value0)
    }

    fn updateUser(&self, value0: std::string::String, value1: std::option::Option<crate::lawspec_data::User>) -> () {
        <abilities::Reference as abilities::UserApi>::updateUser(&reference(&self.resource).model, value0, value1)
    }

    fn deleteUser(&self, value0: std::string::String) -> () {
        <abilities::Reference as abilities::UserApi>::deleteUser(&reference(&self.resource).model, value0)
    }
}
