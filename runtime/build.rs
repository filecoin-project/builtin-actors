static NETWORKS: &[(&[&str], &[&str])] = &[
    (&["mainnet"], &["sector-32g", "sector-64g"]),
    (
        &["caterpillarnet"],
        &[
            "sector-512m",
            "sector-32g",
            "sector-64g",
            "small-deals",
            "short-precommit",
            "min-power-2k",
        ],
    ),
    (&["butterflynet", "wallaby"], &["sector-512m", "sector-32g", "sector-64g", "min-power-2g"]),
    (&["calibrationnet"], &["sector-32g", "sector-64g", "min-power-32g"]),
    (
        &["devnet", "devnet-wasm" /*devnet-fevm*/],
        &["sector-2k", "sector-8m", "small-deals", "short-precommit", "min-power-2k"],
    ),
    (
        &["testing"],
        &[
            "sector-2k",
            "sector-8m",
            "sector-512m",
            "sector-32g",
            "sector-64g",
            "small-deals",
            "short-precommit",
            "min-power-2k",
            "no-provider-deal-collateral",
        ],
    ),
    (
        &["testing-fake-proofs"],
        &[
            "sector-2k",
            "sector-8m",
            "sector-512m",
            "sector-32g",
            "sector-64g",
            "small-deals",
            "short-precommit",
            "min-power-2k",
            "no-provider-deal-collateral",
            "fake-proofs",
        ],
    ),
];
const NETWORK_ENV: &str = "BUILD_FIL_NETWORK";

/// This build script enables _local_ compile features. These features do not
/// affect the dependency graph (they are not processed by Cargo). They are only
/// in effect for conditional compilation _in this crate_.
///
/// The reason we can't set these features as Cargo features from the root build
/// is that this crate is only ever used as a _transitive_ dependency from actor
/// crates.
///
/// So the only two options are: (a) actors crates set the features when
/// importing us as a dependency (super repetitive), or (b) this.
fn main() {
    let network = std::env::var(NETWORK_ENV).ok();
    println!("cargo:rerun-if-env-changed={}", NETWORK_ENV);

    let network = network.as_deref().unwrap_or("mainnet");
    let features = NETWORKS.iter().find(|(k, _)| k.contains(&network)).expect("unknown network").1;
    for feature in features {
        println!("cargo:rustc-cfg=feature=\"{}\"", feature);
    }
}
