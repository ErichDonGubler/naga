use std::{borrow::Cow, convert::TryInto, fmt, num::NonZeroU32};

use crate::{arena::BadHandle, Arena, Handle};

impl super::Validator {
    #[warn(clippy::todo)]
    pub(super) fn validate_module_handles(
        module: &crate::Module,
    ) -> Result<(), InvalidHandleError> {
        let &crate::Module {
            ref constants,
            ref entry_points,
            ref functions,
            ref global_variables,
            ref types,
        } = module;

        // TODO: validate error quality
        fn desc_name_defer_kind<'a, T>(
            name: Option<&'a str>,
            handle: Handle<T>,
        ) -> impl FnOnce(&'static str) -> HandleDescriptor<T, KindAndMaybeName<'a>> {
            move |type_| {
                HandleDescriptor::new(handle, KindAndMaybeName::from_type(type_).with_name(name))
            }
        }

        const fn desc<T>(
            handle: Handle<T>,
            kind: &'static str,
        ) -> HandleDescriptor<T, &'static str> {
            HandleDescriptor::new(handle, kind)
        }

        // NOTE: Types being first is important. All other forms of validation depend on this.
        types
            .iter()
            .try_for_each(|(handle, type_)| -> Result<_, InvalidHandleError> {
                let span = types.get_span(handle);

                let &crate::Type {
                    ref name,
                    ref inner,
                } = type_;
                let this_handle = desc_name_defer_kind(name.as_deref(), handle);

                match inner {
                    &crate::TypeInner::Scalar { .. }
                    | &crate::TypeInner::Vector { .. }
                    | &crate::TypeInner::Matrix { .. }
                    | &crate::TypeInner::ValuePointer { .. }
                    | &crate::TypeInner::Atomic { .. }
                    | &crate::TypeInner::Image { .. }
                    | &crate::TypeInner::Sampler { .. } => Ok(()),
                    &crate::TypeInner::Pointer { base, .. } => this_handle("pointer type")
                        .check_dep(HandleDescriptor::new(base, "base type"))?
                        .ok(),
                    &crate::TypeInner::Array { base, .. } => this_handle("array type")
                        .check_dep(HandleDescriptor::new(base, "base type"))?
                        .ok(),
                    &crate::TypeInner::Struct { ref members, .. } => {
                        let this_handle = this_handle("structure");

                        members
                            .iter()
                            .map(|&crate::StructMember { ref name, ty, .. }| {
                                desc_name_defer_kind(name.as_deref(), ty)("member type")
                            })
                            .try_fold(this_handle, HandleDescriptor::check_dep)?
                            .ok()
                    }
                    &crate::TypeInner::BindingArray { base, .. } => {
                        this_handle("binding array type")
                            .check_dep(HandleDescriptor::new(base, "base type"))?
                            .ok()
                    }
                }
            })?;

        let validate_type = |type_handle| -> Result<(), InvalidHandleError> {
            types.check_contains_handle(type_handle)?;
            Ok(())
        };

        constants
            .iter()
            .try_for_each(|(handle, constant)| -> Result<_, InvalidHandleError> {
                let &crate::Constant {
                    ref name,
                    specialization: _,
                    ref inner,
                } = constant;
                match *inner {
                    crate::ConstantInner::Scalar { .. } => Ok(()),
                    crate::ConstantInner::Composite { ty, ref components } => {
                        validate_type(ty)?;

                        let this_handle = desc_name_defer_kind(name.as_deref(), handle)("constant");
                        components
                            .iter()
                            .copied()
                            .map(|component| desc_name_defer_kind(None, component)("component"))
                            .try_fold(this_handle, HandleDescriptor::check_dep)?
                            .ok()
                    }
                }
            })?;

        let validate_constant = |constant_handle| -> Result<(), InvalidHandleError> {
            constants.check_contains_handle(constant_handle)?;
            Ok(())
        };

        global_variables.iter().try_for_each(
            |(global_variable_handle, global_variable)| -> Result<_, InvalidHandleError> {
                let &crate::GlobalVariable {
                    ref name,
                    space: _,
                    binding: _,
                    ty,
                    init,
                } = global_variable;
                let span = global_variables.get_span(global_variable_handle);
                validate_type(ty)?;
                if let Some(init_expr) = init {
                    validate_constant(init_expr)?;
                }
                Ok(())
            },
        )?;

        let validate_expressions = |expressions: &Arena<crate::Expression>,
                                    local_variables: &Arena<crate::LocalVariable>|
         -> Result<(), InvalidHandleError> {
            expressions
                .iter()
                .try_for_each(|(this_handle, expression)| {
                    let expr = |handle, kind| {
                        HandleDescriptor::new(handle, ExpressionHandleDescription { kind })
                    };
                    let this_expr = |kind| expr(this_handle, kind);
                    let expr_opt = |opt: Option<_>, desc| opt.map(|handle| expr(handle, desc));

                    match expression {
                        &crate::Expression::Access { base, .. }
                        | &crate::Expression::AccessIndex { base, .. } => this_expr("access")
                            .check_dep(expr(base, "access base"))?
                            .ok(),
                        &crate::Expression::Constant(constant) => {
                            validate_constant(constant)?;
                            Ok(())
                        }
                        &crate::Expression::Splat { value, .. } => this_expr("splat")
                            .check_dep(expr(value, "splat value"))?
                            .ok(),
                        &crate::Expression::Swizzle { vector, .. } => {
                            this_expr("swizzle").check_dep(expr(vector, "vector"))?.ok()
                        }
                        &crate::Expression::Compose { ty, ref components } => {
                            validate_type(ty)?;
                            let this_handle = this_expr("composite");
                            components
                                .iter()
                                .copied()
                                .map(|component| expr(component, "component"))
                                .try_fold(this_handle, HandleDescriptor::check_dep)?
                                .ok()
                        }
                        // TODO: Should we validate the length of function args?
                        &crate::Expression::FunctionArgument(_arg_idx) => Ok(()),
                        &crate::Expression::GlobalVariable(global_variable) => {
                            global_variables.check_contains_handle(global_variable)?;
                            Ok(())
                        }
                        &crate::Expression::LocalVariable(local_variable) => {
                            // TODO: Shouldn't we be checking for forward deps here, too?
                            local_variables.check_contains_handle(local_variable)?;
                            Ok(())
                        }
                        &crate::Expression::Load { pointer } => {
                            // TODO: right naming?
                            this_expr("load").check_dep(expr(pointer, "pointee"))?.ok()
                        }
                        &crate::Expression::ImageSample {
                            image,
                            sampler,
                            gather: _,
                            coordinate,
                            array_index,
                            offset,
                            level: _,
                            depth_ref,
                        } => {
                            // TODO: is there a better order for validation?

                            if let Some(offset) = offset {
                                validate_constant(offset)?;
                            }

                            this_expr("image sample")
                                .check_dep(expr(image, "image"))?
                                .check_dep(expr(sampler, "sampler"))? // TODO: Is this name correct? :think:
                                .check_dep(expr(coordinate, "coordinate"))?
                                .check_dep_opt(expr_opt(array_index, "array index"))?
                                .check_dep_opt(expr_opt(depth_ref, "depth reference"))?
                                .ok()
                        }
                        &crate::Expression::ImageLoad {
                            image,
                            coordinate,
                            array_index,
                            sample,
                            level,
                        } => this_expr("image load")
                            .check_dep(expr(image, "image"))?
                            .check_dep(expr(coordinate, "coordinate"))?
                            .check_dep_opt(expr_opt(array_index, "array index"))?
                            .check_dep_opt(expr_opt(sample, "sample index"))?
                            .check_dep_opt(expr_opt(level, "level of detail"))?
                            .ok(),
                        &crate::Expression::ImageQuery { image, query } => this_expr("image query")
                            .check_dep(expr(image, "image"))?
                            .check_dep_opt(match query {
                                crate::ImageQuery::Size { level } => {
                                    expr_opt(level, "level of detail")
                                }
                                crate::ImageQuery::NumLevels
                                | crate::ImageQuery::NumLayers
                                | crate::ImageQuery::NumSamples => None,
                            })?
                            .ok(),
                        &crate::Expression::Unary {
                            op: _,
                            expr: operand,
                        } => this_expr("unary")
                            // TODO: maybe use operator names?
                            .check_dep(expr(operand, "unary operand"))?
                            .ok(),
                        &crate::Expression::Binary { op: _, left, right } => this_expr("binary")
                            // TODO: maybe use operator names?
                            .check_dep(expr(left, "left operand"))?
                            .check_dep(expr(right, "right operand"))?
                            .ok(),
                        &crate::Expression::Select {
                            condition,
                            accept,
                            reject,
                        } => desc(this_handle, "`select` function call") // TODO: use function name/more platform-generic name?
                            .check_dep(expr(condition, "condition"))?
                            .check_dep(expr(accept, "accept"))?
                            .check_dep(expr(reject, "reject"))?
                            .ok(),
                        &crate::Expression::Derivative {
                            axis: _,
                            expr: argument,
                        } => {
                            // TODO: use function name/more platform-generic name?
                            this_expr("derivative")
                                .check_dep(expr(argument, "argument"))?
                                .ok()
                        }
                        &crate::Expression::Relational { fun: _, argument } => {
                            // TODO: use function name/more platform-generic name?
                            desc(this_handle, "relational function call")
                                .check_dep(expr(argument, "argument"))?
                                .ok()
                        }
                        &crate::Expression::Math {
                            fun: _,
                            arg,
                            arg1,
                            arg2,
                            arg3,
                        } => {
                            // TODO: use function name/more platform-generic name?
                            desc(this_handle, "math function call")
                                .check_dep(expr(arg, "first argument"))?
                                .check_dep_opt(expr_opt(arg1, "second argument"))?
                                .check_dep_opt(expr_opt(arg2, "third argument"))?
                                .check_dep_opt(expr_opt(arg3, "fourth argument"))?
                                .ok()
                        }
                        &crate::Expression::As {
                            expr: input,
                            kind: _,
                            convert: _,
                        } => {
                            // TODO: use `kind` (ex., "cast to ...")?
                            this_expr("cast").check_dep(expr(input, "input"))?.ok()
                        }
                        &crate::Expression::CallResult(function) => {
                            functions.check_contains_handle(function)?;
                            Ok(())
                        }
                        &crate::Expression::AtomicResult { .. } => Ok(()),
                        &crate::Expression::ArrayLength(array) => this_expr("array length")
                            .check_dep(expr(array, "array"))?
                            .ok(),
                    }
                })
        };

        let validate_function = |span, function: &_| -> Result<(), InvalidHandleError> {
            let &crate::Function {
                name: _,
                ref arguments,
                ref result,
                ref local_variables,
                ref expressions,
                ref named_expressions,
                ref body,
            } = function;

            local_variables.iter().try_for_each(
                |(handle, local_variable)| -> Result<_, InvalidHandleError> {
                    let &crate::LocalVariable { ref name, ty, init } = local_variable;
                    validate_type(ty)?;
                    if let Some(init_constant) = init {
                        // TODO: wait, where's the context? :(
                        validate_constant(init_constant)?;
                    }
                    Ok(())
                },
            )?;

            validate_expressions(expressions, local_variables)?;
            Ok(())
        };

        entry_points
            .iter()
            .try_for_each(|entry_point| -> Result<_, InvalidHandleError> {
                // TODO: Why don't we have a `handle`/`Span` here?
                validate_function(crate::Span::default(), &entry_point.function)
            })?;

        functions.iter().try_for_each(
            |(function_handle, function)| -> Result<_, InvalidHandleError> {
                let span = functions.get_span(function_handle);
                validate_function(span, function)
            },
        )?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct KindAndMaybeName<'a> {
    kind: &'static str,
    name: Option<Cow<'a, str>>,
}

impl<'a> KindAndMaybeName<'a> {
    pub const fn from_type(type_: &'static str) -> Self {
        Self {
            kind: type_,
            name: None,
        }
    }

    pub fn with_name<'b>(self, name: Option<impl Into<Cow<'b, str>>>) -> KindAndMaybeName<'b> {
        let Self {
            kind: type_,
            name: _,
        } = self;

        KindAndMaybeName {
            kind: type_,
            name: name.map(Into::into),
        }
    }

    pub fn into_static(self) -> KindAndMaybeName<'static> {
        let Self { kind: type_, name } = self;

        KindAndMaybeName {
            kind: type_,
            name: name.map(|n| n.into_owned().into()),
        }
    }
}

impl fmt::Display for KindAndMaybeName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let &Self { ref kind, ref name } = self;
        write!(f, "{kind}")?;
        if let Some(name) = name.as_ref() {
            write!(f, " {name:?}")?;
        }
        Ok(())
    }
}

impl HandleDescription for KindAndMaybeName<'_> {
    fn into_erased(self) -> Box<dyn HandleDescription + 'static> {
        Box::new(self.into_static())
    }
}

#[derive(Clone, Debug)]
struct ExpressionHandleDescription {
    kind: &'static str,
}

impl fmt::Display for ExpressionHandleDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let &Self { kind } = self;
        write!(f, "{kind} expression")
    }
}

impl HandleDescription for ExpressionHandleDescription {
    fn into_erased(self) -> Box<dyn HandleDescription + 'static> {
        Box::new(self)
    }
}

// TODO: use a more concrete model for better diagnostics?
#[derive(Debug, thiserror::Error)]
pub enum InvalidHandleError {
    #[error(transparent)]
    Bad(#[from] BadHandle),
    #[error(transparent)]
    ForwardDependency(#[from] FwdDepError),
}

// TODO: use a more concrete model for better diagnostics?
#[derive(Debug, thiserror::Error)]
#[error("{subject} depends on {depends_on}, which has not been processed yet")]
pub struct FwdDepError {
    // TODO: context of what's being validated?
    subject: HandleDescriptor<(), Box<dyn HandleDescription>>,
    depends_on: HandleDescriptor<(), Box<dyn HandleDescription>>,
}

#[derive(Clone, Copy, Debug)]
pub struct HandleDescriptor<T, D> {
    pub(crate) handle: Handle<T>,
    pub(crate) description: D,
    // TODO: track type name?
}

impl<T, D> HandleDescriptor<T, D> {
    pub const fn new(handle: Handle<T>, description: D) -> Self {
        Self {
            handle,
            description,
        }
    }

    pub fn description_mut(&mut self) -> &mut D {
        &mut self.description
    }
}

impl<T, D> HandleDescriptor<T, D>
where
    D: HandleDescription,
{
    /// Check that `self`'s handle is valid for `arena`.
    ///
    /// As with all [`Arena`] handles, it is the responsibility of the caller to ensure that
    /// `self`'s handle is valid for the provided `arena`. Otherwise, the result
    pub(self) fn check_valid_for(self, arena: &Arena<T>) -> Result<Self, InvalidHandleError> {
        arena.check_contains_handle(self.handle)?;
        Ok(self)
    }

    /// Check that `depends_on`'s handle is "ready" to be consumed by `self`'s handle by comparing
    /// handle indices. If `self` describes a valid value (i.e., it has been validated using
    /// [`Self::is_good_in`] and this function returns [`Ok`], then it may be assumed that
    /// `depends_on` also passes that validation.
    ///
    /// In [`naga`](crate)'s current arena-based implementation, this is useful for validating
    /// recursive definitions of arena-based values in linear time.
    ///
    /// As with all [`Arena`] handles, it is the responsibility of the caller to ensure that `self`
    /// and `depends_on` contain handles from the same arena. Otherwise, calling this likely isn't
    /// correct!
    ///
    /// # Errors
    ///
    /// If `depends_on`'s handle is from the same [`Arena`] as `self'`s, but not constructed earlier
    /// than `self`'s, this function returns an error.
    pub(self) fn check_dep<D2>(
        self,
        depends_on: HandleDescriptor<T, D2>,
    ) -> Result<Self, FwdDepError>
    where
        D2: HandleDescription,
    {
        if depends_on.handle < self.handle {
            Ok(self)
        } else {
            Err(FwdDepError {
                subject: self.into_erased(),
                depends_on: depends_on.into_erased(),
            })
        }
    }

    /// Like [`Self::check_dep`], except for [`Optional`] handle values.
    pub(self) fn check_dep_opt<D2>(
        self,
        depends_on: Option<HandleDescriptor<T, D2>>,
    ) -> Result<Self, FwdDepError>
    where
        D2: HandleDescription,
    {
        if let Some(depends_on) = depends_on {
            self.check_dep(depends_on)
        } else {
            Ok(self)
        }
    }

    fn into_erased(self) -> HandleDescriptor<(), Box<dyn HandleDescription>> {
        let Self {
            handle,
            description,
        } = self;

        HandleDescriptor {
            handle: Handle::new(NonZeroU32::new(handle.index().try_into().unwrap()).unwrap()),
            description: description.into_erased(),
        }
    }

    /// Finishes a chain of checks done with this handle descriptor with [`Ok`].
    ///
    /// This API exists because the current method API design favors chained `?` calls. It's often
    /// more convenient to write:
    ///
    /// ```
    /// # fn main() -> Result<(), InvalidHandleError> {
    /// # let first_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let second_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let third_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let fourth_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let fifth_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// first_handle
    ///     .check_dep(second_handle)?
    ///     .check_dep(third_handle)?
    ///     .check_dep(fourth_handle)?
    ///     .check_dep(fifth_handle)?
    ///     .ok() // requires no type inference, single expression
    /// # }
    /// ```
    ///
    /// ...than this:
    ///
    /// ```
    /// # fn main() -> Result<(), InvalidHandleError> {
    /// # let first_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let second_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let third_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let fourth_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// # let fifth_handle = HandleDescriptor::new(Handle::new(0), "asdf");
    /// first_handle
    ///     .check_dep(second_handle)?
    ///     .check_dep(third_handle)?
    ///     .check_dep(fourth_handle)?
    ///     .check_dep(fifth_handle)?;
    /// Ok(()) // may require explicit type specification to use `?`, requires a block
    /// # }
    /// ```
    #[allow(clippy::missing_const_for_fn)] // NOTE: This fires incorrectly without this. :<
    pub(self) fn ok(self) -> Result<(), InvalidHandleError> {
        Ok(())
    }
}

impl<T, D> fmt::Display for HandleDescriptor<T, D>
where
    D: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let &Self {
            ref handle,
            ref description,
        } = self;
        write!(f, "{description} (handle {handle:?})")
    }
}

// impl PartialEq for HandleDescriptor {
//     fn eq(&self, other: &Self) -> bool {
//         self.handle.eq(&other.handle)
//     }
// }

pub trait HandleDescription
where
    Self: fmt::Debug + fmt::Display,
{
    fn into_erased(self) -> Box<dyn HandleDescription>;
}

impl HandleDescription for Box<dyn HandleDescription> {
    fn into_erased(self) -> Box<dyn HandleDescription> {
        self
    }
}

impl HandleDescription for &'static str {
    fn into_erased(self) -> Box<dyn HandleDescription> {
        Box::new(self)
    }
}
