# hello-node

This is a demo for adding a custom node via using `aerie` as a dependency.

Dependency overrides or rig and snarl are needed until issues are fixed
upstream. But other dependencies are optional if you don't need them in
your main function.

> [!IMPORTANT]
> Other builds without your custom node will not be able to load workflows
> generate by your build. You should ensure a unique data directory is used.
> This can be done by setting the app name in the builder or by supplying
> a `data_dir_fn` hook.
