//! `lawtwin import openapi` on the vendored documents and the fixtures.
//! These check what the importer writes; `make contract-test` checks that
//! LawSpec compiles it and that its laws pass.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lawtwin-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn import(document: &Path, package: &str, out: &Path, extra: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_lawtwin"))
        .args(["import", "openapi"])
        .arg(document)
        .args(["--package", package, "--out"])
        .arg(out)
        .args(extra)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap()
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The Swagger Petstore: types, abilities, resources, laws and natives.
#[test]
fn petstore_3_0_json() {
    let out = scratch("petstore");
    let said = import(
        &repo().join("openapi/petstore3.json"),
        "lawtwin.petstore",
        &out,
        &[],
    );
    assert!(
        said.contains("19 operation(s) in 3 abilities, 3 resource(s)"),
        "{said}"
    );
    let model = read(out.join("src/model.lawspec"));
    let api = read(out.join("src/api.lawspec"));
    // Objects are records, enums sums, optional fields Maybe.
    assert!(model.contains("type Pet is a record{\n    id :: Maybe Int64\n    name :: Text\n"));
    assert!(model.contains("type PetStatus is one of\n  | Available -- \"available\""));
    // An inline enum with the values of a named one is that one.
    assert!(api.contains("findPetsByStatus :: PetStatus -> List Pet fails with ApiError"));
    // One ability per tag, with typed failures.
    assert!(api.contains("ability PetApi with Service is"));
    assert!(api.contains("ability StoreApi with Service is"));
    assert!(model.contains("  | UnprocessableEntity -- HTTP 422"));
    // A pet's key comes from the body, or the service assigns it.
    assert!(model.contains("let key = (first matching case of body.id : when Present given then given when Absent then current_.nextPetKey end case) in"));
    assert!(api.contains("law \"|a created pet reads back| is"));
    assert!(api.contains("law \"|a deleted pet is not found| is"));
    assert!(api.contains("law \"|a replaced pet reads back as replaced| is"));
    // A user's key must be given.
    assert!(api.contains("(body with username = Present keyGiven)"));
    // What the importer cannot model is left to the author.
    assert!(
        api.contains("operation findPetsByStatus by handwritten.findPetsByStatus end operation")
    );
    assert!(
        read(out.join("src/handwritten.lawspec"))
            .contains("procedure findPetsByStatus (status :: model.PetStatus) (current_ :: State)")
    );
    // Native handlers for the test project on Rust and Python.
    assert!(
        read(out.join("test/rust/src/lawtwin/petstore.rs"))
            .contains("impl abilities::PetApi for PetApiHandler")
    );
    assert!(
        read(out.join("test/python/src/lawtwin/petstore/__init__.py"))
            .contains("class PetApiHandler(_Reference)")
    );
    let configuration: edn_rs::Edn = read(out.join("test/lawspec.edn")).parse().unwrap();
    assert_eq!(configuration[":manifest-version"].to_int(), Some(1));
    assert_eq!(configuration[":language-profile"], edn_rs::Edn::Str("1.0-rc.2".into()));
    assert!(
        configuration[":verification"][":layers"]
            .to_vec()
            .unwrap()
            .iter()
            .any(|layer| layer.trim_start_matches(':') == "property")
    );
    let adapter = read(out.join("lawtwin-embed.rs"));
    assert!(adapter.contains("self.handler.try_addPet("));
    assert!(!adapter.contains("lawspec_definitions"));
    let map: serde_json::Value =
        serde_json::from_str(&read(out.join("lawtwin-openapi.json"))).unwrap();
    assert_eq!(map["resources"].as_array().unwrap().len(), 3);
}

/// The YAML Petstore: arrays as types, and a key the importer does not guess.
#[test]
fn petstore_3_0_yaml() {
    let out = scratch("petstore-yaml");
    let said = import(
        &repo().join("crates/lawtwin/tests/fixtures/petstore.yaml"),
        "petstore",
        &out,
        &[],
    );
    assert!(
        said.contains("3 operation(s) in 1 ability, 1 resource(s)"),
        "{said}"
    );
    let model = read(out.join("src/model.lawspec"));
    let api = read(out.join("src/api.lawspec"));
    // The `Pets` schema is an array, so it is no type of its own.
    assert!(api.contains("listPets :: Maybe Int32 -> List Pet fails with ApiError"));
    assert!(model.contains(
        "procedure modelShowPetById (petId :: Text) (current_ :: State) -> Pair Pet State"
    ));
    // The path names a pet by text, but a pet's id is an integer: the
    // importer does not guess how they meet, and leaves the create alone.
    assert!(api.contains("createPets by handwritten.createPets"));
}

/// OpenAPI 3.1's nullability, unions, maps, constraints and status ranges.
#[test]
fn features_of_3_1() {
    let out = scratch("tour");
    import(
        &repo().join("crates/lawtwin/tests/fixtures/features-3.1.yaml"),
        "tour",
        &out,
        &[],
    );
    let model = read(out.join("src/model.lawspec"));
    let api = read(out.join("src/api.lawspec"));
    // `type: [string, "null"]` is Maybe; maps are KeyVal Text; allOf merges.
    assert!(model.contains("    body :: Maybe Text\n"));
    assert!(model.contains("    meta :: Maybe (KeyVal Text Text)\n"));
    assert!(model.contains("type Note is a record{\n    title :: Text\n"));
    assert!(model.contains("    created :: Text\n"));
    // oneOf is a sum with a constructor per alternative; a null one is Maybe.
    assert!(model.contains("  | HitNote of type Note"));
    assert!(api.contains("search :: Text -> Maybe Hit fails with ApiError"));
    // Bounds, lengths, patterns and formats are refinements and validation.
    assert!(model.contains("refinement NewNoteTitle is (v :: Text where prelude.length v >= 1 && prelude.length v <= 80) end refinement"));
    assert!(model.contains("refinement NewNotePriority is (v :: Float64 where prelude.isFinite v && v > 0.0 && v <= 10.0) end refinement"));
    assert!(model.contains("item matches regex \"[a-z0-9-]+\""));
    assert!(model.contains("if !(validNewNote body) then raise UnprocessableEntity else"));
    // Status ranges are failures too.
    assert!(model.contains("  | ClientError -- HTTP 4XX"));
    // A patch changes only the fields it gives.
    assert!(api.contains("law \"|an update changes only the fields it gives to a note| is"));
    assert!(model.contains(
        "title = (first matching case of change.title : when Absent then old.title when Present given then given end case)"
    ));
    assert!(api.contains("law \"|the list holds a created note| is"));
}

/// A second import keeps the hand-written unit and names what it lacks.
#[test]
fn hand_written_units_are_kept() {
    let out = scratch("kept");
    let doc = repo().join("openapi/petstore3.json");
    import(&doc, "lawtwin.petstore", &out, &["--tag", "pet"]);
    let hand = out.join("src/handwritten.lawspec");
    std::fs::write(
        &hand,
        "unit lawtwin.petstore.handwritten\n-- mine\nend unit\n",
    )
    .unwrap();
    let said = import(&doc, "lawtwin.petstore", &out, &["--tag", "pet"]);
    assert_eq!(
        read(hand),
        "unit lawtwin.petstore.handwritten\n-- mine\nend unit\n"
    );
    assert!(
        said.contains("add a definition for each of: findPetsByStatus"),
        "{said}"
    );
}

/// `--path` keeps the operations under a path; parents make a key record.
#[test]
fn paths_select_operations() {
    let out = scratch("paths");
    let said = import(
        &repo().join("openapi/github-labels-milestones.json"),
        "lawtwin.github",
        &out,
        &["--path", "/repos/{owner}/{repo}/labels"],
    );
    assert!(
        said.contains("5 operation(s) in 1 ability, 1 resource(s)"),
        "{said}"
    );
    let model = read(out.join("src/model.lawspec"));
    // A resource under parents is kept by a key record of the path's names.
    assert!(model.contains(
        "type LabelKey is a record{\n    owner :: Text\n    repo :: Text\n    name :: Text\n}\nend type"
    ));
    assert!(model.contains("    nodeId :: Text -- `node_id`"));
    assert!(model.contains("    default_ :: Bool -- `default`"));
}

/// The version-2 default selects no law checks. Our conformance projects must
/// explicitly select property execution so an upgrade cannot silently turn the
/// native acceptance command into an intrinsic-validation-only success.
#[test]
fn maintained_contract_projects_select_law_execution() {
    for contract in ["kafka", "petstore", "github"] {
        let config: edn_rs::Edn = read(repo().join(format!("contracts/{contract}/test/lawspec.edn"))).parse().unwrap();
        assert_eq!(config[":manifest-version"].to_int(), Some(1), "{contract}");
        assert_eq!(config[":language-profile"], edn_rs::Edn::Str("1.0-rc.2".into()), "{contract}");
        let layers = config[":verification"][":layers"]
            .to_vec()
            .expect("explicit verification layers");
        assert!(
            layers.iter().any(|value| value.trim_start_matches(':') == "property"),
            "{contract}: unbounded laws need native observations"
        );
    }
}
