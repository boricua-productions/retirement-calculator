# Windows Dependencies Analysis

## Summary

The project currently has **4 versions** of `windows-sys` and **2 versions** of `windows-targets` in the dependency tree. This is a known and acceptable situation in the Rust ecosystem during transition periods.

## Current State (as of 2026-05-18)

### windows-sys versions:
- **v0.52.0** — Used by: `eframe`, `glutin`, `glutin_egl_sys`, `winit`
- **v0.59.0** — Used by: `rfd` (file dialog crate)
- **v0.60.2** — Used by: `arboard` (clipboard via egui-winit)
- **v0.61.2** — Used by: `anstyle-query`, `anstyle-wincon`, `tempfile` (dev-dependencies)

### windows-targets versions:
- **v0.52.6** — Transitive dependency through windows-sys 0.52.0
- **v0.53.5** — Transitive dependency through windows-sys 0.60.2

## Why This Exists

These are **transitive dependencies** from our GUI framework stack:
- `eframe v0.29` → requires `windows-sys 0.52.0`
- `rfd v0.15.4` → requires `windows-sys 0.59.0`
- `arboard v3.6.1` → requires `windows-sys 0.60.2`

Each upstream crate specifies its own `windows-sys` version requirement in its Cargo.toml, and we cannot override these without patching from alternative sources (Git repos, local paths).

## Why We Can't Fix It Now

### Attempted Solution: Upgrade to eframe 0.34

We attempted to upgrade from eframe 0.29 → 0.34 to consolidate Windows dependencies. This failed because:

1. **Breaking API changes** in egui 0.34:
   - `Line::new()` now requires 2 arguments (name + series) instead of 1
   - `BarChart::new()` now requires 2 arguments (name + bars) instead of 1
   - `Margin::same()` now expects `i8` instead of `f64`
   - `Frame::none()` deprecated in favor of `Frame::NONE` or `Frame::new()`
   - Panel methods renamed/deprecated (`show()` → `show_inside()`)
   - Multiple `ecolor` and `egui` version conflicts

2. **Scope of changes required**:
   - 25+ compilation errors across multiple UI files
   - Would require refactoring chart_panel.rs, input_panel.rs, and app.rs
   - Risk of introducing UI bugs during migration

### Alternative: cargo patch

Cannot use `[patch.crates-io]` to force all crates to use a single version because:
- Patches must point to **different sources** (Git repos, local paths)
- Cannot patch a crates.io dependency to another version on crates.io
- This is a Cargo design limitation

## Impact Assessment

### Binary Size
- **Impact**: Minimal (~1-2MB for Windows platform)
- `windows-sys` is a thin wrapper over Windows FFI bindings
- Most of the "duplication" is in compile artifacts, not runtime

### Compile Time
- **Impact**: Slightly increased (few extra seconds)
- Each version must be compiled separately
- Not significant for a project of this size

### Runtime Performance
- **Impact**: None
- Different versions don't conflict at runtime
- Each crate uses its own version in isolation
- No performance degradation

### Maintenance
- **Impact**: No immediate action required
- Common situation during ecosystem transitions
- Will naturally resolve as upstream crates update

## Future Resolution

The Rust ecosystem is converging toward `windows-sys 0.61.2` (latest stable). As upstream crates update, duplication will decrease naturally.

**Timeline estimates:**
- `eframe 0.34+` likely consolidates to newer windows-sys
- `rfd` and `arboard` will follow
- Expected consolidation: 6-12 months as crates migrate

## Recommendation

**Accept the current duplication.** The cost/benefit analysis favors waiting:

**Costs of upgrading now:**
- 2-3 days of refactoring UI code
- Risk of introducing regressions
- Ongoing maintenance burden if egui APIs continue evolving

**Benefits of waiting:**
- Upstream crates will naturally converge
- Future egui upgrades may include automated migration tools
- No runtime impact from current duplication

## Verification Commands

```bash
# Show all duplicate dependencies
cargo tree -d

# Show which crates depend on a specific windows-sys version
cargo tree -i windows-sys@0.52.0
cargo tree -i windows-sys@0.59.0
cargo tree -i windows-sys@0.60.2
cargo tree -i windows-sys@0.61.2

# Check for outdated dependencies
cargo outdated
```

## References

- [windows-sys crate](https://crates.io/crates/windows-sys)
- [egui 0.29 → 0.34 migration guide](https://github.com/emilk/egui/blob/master/CHANGELOG.md)
- [Cargo patch documentation](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
