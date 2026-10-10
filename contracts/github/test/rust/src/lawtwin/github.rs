// Written by `lawtwin import openapi`, and written again on every import.
//! The native handlers of lawtwin.github, for running its laws: each answers
//! through the reference model, held by the case's Service. A connection
//! to the HTTP twin or to the real service takes its place in conformance
//! runs.
#![allow(unused_variables, unused_imports, non_snake_case)]
use crate::lawspec_abilities::lawtwin_github as abilities;
use crate::lawspec_runtime as ls;
use std::sync::Arc;

/// What a Service holds: the reference model.
pub struct ReferenceService {
    model: Arc<abilities::Reference>,
}

fn reference(service: &crate::lawspec_data::Service) -> &ReferenceService {
    service.native::<ReferenceService>().unwrap_or_else(|error| panic!("{error}"))
}

// LawSpec: (Unit -> lawtwin.github::type::Service)
pub fn connectService(value0: ()) -> crate::lawspec_data::Service {
    ls::Handle::new(ReferenceService { model: Arc::new(abilities::Reference::new(&ls::Context::default())) })
}

// LawSpec: (lawtwin.github::type::Service -> Unit)
pub fn disconnectService(value0: crate::lawspec_data::Service) -> () {}

/// IssuesApi, answered by the reference model.
pub struct IssuesApiHandler {
    pub resource: crate::lawspec_data::Service,
}

impl IssuesApiHandler {
    pub fn new(resource: crate::lawspec_data::Service) -> Self {
        Self { resource }
    }
}

impl abilities::IssuesApi for IssuesApiHandler {
    fn issuesListLabelsForRepo(&self, value0: std::string::String, value1: std::string::String, value2: std::option::Option<i64>, value3: std::option::Option<i64>) -> std::vec::Vec<crate::lawspec_data::Label> {
        <abilities::Reference as abilities::IssuesApi>::issuesListLabelsForRepo(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn issuesCreateLabel(&self, value0: std::string::String, value1: std::string::String, value2: crate::lawspec_data::IssuesCreateLabelBody) -> crate::lawspec_data::Label {
        <abilities::Reference as abilities::IssuesApi>::issuesCreateLabel(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesGetLabel(&self, value0: std::string::String, value1: std::string::String, value2: std::string::String) -> crate::lawspec_data::Label {
        <abilities::Reference as abilities::IssuesApi>::issuesGetLabel(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesDeleteLabel(&self, value0: std::string::String, value1: std::string::String, value2: std::string::String) -> () {
        <abilities::Reference as abilities::IssuesApi>::issuesDeleteLabel(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesUpdateLabel(&self, value0: std::string::String, value1: std::string::String, value2: std::string::String, value3: std::option::Option<crate::lawspec_data::IssuesUpdateLabelBody>) -> crate::lawspec_data::Label {
        <abilities::Reference as abilities::IssuesApi>::issuesUpdateLabel(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn issuesListMilestones(&self, value0: std::string::String, value1: std::string::String, value2: std::option::Option<crate::lawspec_data::IssuesListMilestonesState>, value3: std::option::Option<crate::lawspec_data::IssuesListMilestonesSort>, value4: std::option::Option<crate::lawspec_data::IssuesListMilestonesDirection>, value5: std::option::Option<i64>, value6: std::option::Option<i64>) -> std::vec::Vec<crate::lawspec_data::Milestone> {
        <abilities::Reference as abilities::IssuesApi>::issuesListMilestones(&reference(&self.resource).model, value0, value1, value2, value3, value4, value5, value6)
    }

    fn issuesCreateMilestone(&self, value0: std::string::String, value1: std::string::String, value2: crate::lawspec_data::IssuesCreateMilestoneBody) -> crate::lawspec_data::Milestone {
        <abilities::Reference as abilities::IssuesApi>::issuesCreateMilestone(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesGetMilestone(&self, value0: std::string::String, value1: std::string::String, value2: i64) -> crate::lawspec_data::Milestone {
        <abilities::Reference as abilities::IssuesApi>::issuesGetMilestone(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesDeleteMilestone(&self, value0: std::string::String, value1: std::string::String, value2: i64) -> () {
        <abilities::Reference as abilities::IssuesApi>::issuesDeleteMilestone(&reference(&self.resource).model, value0, value1, value2)
    }

    fn issuesUpdateMilestone(&self, value0: std::string::String, value1: std::string::String, value2: i64, value3: std::option::Option<crate::lawspec_data::IssuesUpdateMilestoneBody>) -> crate::lawspec_data::Milestone {
        <abilities::Reference as abilities::IssuesApi>::issuesUpdateMilestone(&reference(&self.resource).model, value0, value1, value2, value3)
    }
}
