use clap::Parser;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[clap(subcommand)]
    pub subcommand: Subcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum Subcommand {
    All,
    Bench {
        #[clap(long)]
        clean: bool,
    },
    #[clap(subcommand)]
    Validate(ValidateSubcommand),
}

#[derive(Debug, Parser)]
pub(crate) enum ValidateSubcommand {
    #[clap(name = "spv")]
    Spirv,
    #[clap(name = "msl")]
    Metal,
    Glsl,
    Dot,
    Wgsl,
    #[clap(subcommand)]
    Hlsl(ValidateHlslCommand),
}

#[derive(Debug, Parser)]
pub(crate) enum ValidateHlslCommand {
    Dxc,
    Fxc,
}
