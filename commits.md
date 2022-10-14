- [ ] Split out `ExprFedDepError`
- [ ] Combine handle validation pass errors at current implementation level
- [ ] Split out the pass!
	- [ ] squash! add `ExpressionTypeResolver::check`
	- [ ] squash! `impl Index<Expression> for ExpressionTypeResolver`
	- [ ] note: trade-offs were made, we're now doing multiple passes. Boo for
		perf, yay for abstraction.
