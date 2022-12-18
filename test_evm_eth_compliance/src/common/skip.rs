use crate::statetest::SpecName;
use serde::Deserialize;
use std::collections::BTreeMap;

lazy_static::lazy_static! {
    pub static ref SKIP_TESTS: SkipTests = {
        let skip_data = include_bytes!("../../test-vectors/skip_test.json");
        SkipTests::load(&skip_data[..]).expect("JSON from disk is Invalid")
    };
}

/// Test to skip (only if issue ongoing)
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct SkipTests {
    /// State tests
    pub state: Vec<SkipStateTest>,
}

/// State test to skip.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipStateTest {
    /// Issue ID.
    pub issue_id: String,
    /// Test failing name.
    pub failing: String,
    /// Items failing from the pre block.
    pub pre_tests: Option<BTreeMap<String, StateSkipPreTest>>,
    /// Items failing from the post block.
    pub post_tests: Option<BTreeMap<String, StateSkipPostTest>>,
}

/// State subtest to skip.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSkipPreTest {
    /// Pre State owner address of this item. Or '*' for all state.
    pub pre_owners: Vec<String>,
}

/// State subtest to skip.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSkipPostTest {
    /// Post State test number of this item. Or '*' for all state.
    pub sub_numbers: Vec<String>,
    /// Chain for this items.
    pub chain_spec: SpecName,
}

impl SkipTests {
    /// Empty skip states.
    pub fn empty() -> Self {
        SkipTests { state: Vec::new() }
    }

    /// Loads test from json.
    pub fn load<R>(reader: R) -> Result<Self, serde_json::Error>
    where
        R: std::io::Read,
    {
        serde_json::from_reader(reader)
    }
}
