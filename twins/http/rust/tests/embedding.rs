//! The public handler adapter must preserve state across a refusal and keep
//! trial models isolated. These are the server guarantees previously handled
//! by scraped typed-definition calls.
use lawtwin_http::contracts;
use lawtwin_http::model::Outcome;
use lawtwin_wire::V;

fn pet(id: i64) -> V {
    V::Data {
        ty: "Pet".into(),
        ctor: "Pet".into(),
        fields: vec![
            V::Maybe(Some(Box::new(V::Int(id)))),
            V::Text("Rex".into()),
            V::Maybe(None),
            V::List(vec![]),
            V::Maybe(None),
            V::Maybe(None),
        ],
    }
}

#[test]
fn refusals_leave_state_and_the_handler_usable() {
    let mut model = (contracts::find("petstore").unwrap().start)();
    let before = model.state();
    assert_eq!(
        model.call("getPetById", vec![V::Int(91)]),
        Err(Outcome::Failure("NotFound".into()))
    );
    assert_eq!(model.state(), before);
    let created = pet(91);
    assert_eq!(
        model.call("addPet", vec![created.clone()]),
        Ok(created.clone())
    );
    assert_eq!(model.call("getPetById", vec![V::Int(91)]), Ok(created));
}

#[test]
fn trial_models_have_independent_state_after_a_shared_snapshot() {
    let mut model = (contracts::find("petstore").unwrap().start)();
    model.call("addPet", vec![pet(91)]).unwrap();
    let mut trial = model.boxed();
    trial
        .call("deletePet", vec![V::Int(91), V::Maybe(None)])
        .unwrap();
    assert_eq!(
        trial.call("getPetById", vec![V::Int(91)]),
        Err(Outcome::Failure("NotFound".into()))
    );
    assert_eq!(model.call("getPetById", vec![V::Int(91)]), Ok(pet(91)));
}

#[test]
fn malformed_calls_are_defects_and_cannot_change_state() {
    let mut model = (contracts::find("petstore").unwrap().start)();
    let before = model.state();
    for (op, args) in [
        ("addPet", vec![]),
        ("addPet", vec![pet(1), pet(2)]),
        ("getPetById", vec![V::Text("wrong type".into())]),
        ("absent", vec![]),
    ] {
        assert!(matches!(model.call(op, args), Err(Outcome::Defect(_))));
        assert_eq!(model.state(), before);
    }
}
