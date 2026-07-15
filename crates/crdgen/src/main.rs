//! Emits `deploy/crd/*.yaml`, one file per CRD kind, via
//! `CustomResourceExt::crd()`. Invoked directly by `task recu` — see
//! `.prompt/plan.md`, "Architecture hexagonale".

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use kube::CustomResourceExt;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "deploy/crd")]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.out)?;

    write_crd(
        &args.out,
        "authentikinstance",
        api::AuthentikInstance::crd(),
    )?;
    write_crd(&args.out, "authentikgroup", api::AuthentikGroup::crd())?;
    write_crd(&args.out, "authentikoutpost", api::AuthentikOutpost::crd())?;
    write_crd(&args.out, "authentikuser", api::AuthentikUser::crd())?;
    write_crd(&args.out, "authentikbrand", api::AuthentikBrand::crd())?;
    write_crd(
        &args.out,
        "authentikapplication",
        api::AuthentikApplication::crd(),
    )?;
    write_crd(
        &args.out,
        "authentikaccesspolicy",
        api::AuthentikAccessPolicy::crd(),
    )?;
    write_crd(
        &args.out,
        "authentiknamespacepolicy",
        api::AuthentikNamespacePolicy::crd(),
    )?;

    println!("wrote CRDs to {}", args.out.display());
    Ok(())
}

fn write_crd(out: &Path, name: &str, crd: impl serde::Serialize) -> anyhow::Result<()> {
    let yaml = serde_norway::to_string(&crd)?;
    fs::write(out.join(format!("{name}.yaml")), yaml)?;
    Ok(())
}
