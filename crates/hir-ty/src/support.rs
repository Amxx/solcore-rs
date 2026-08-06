use hir::anchor::DefId;
use nameres::{
    LibraryId, ModuleId, module_id_for_source_file, module_id_from_key, module_key_for_path,
    reachable_modules,
};

use crate::Db;

pub(crate) fn module_for_def_via_graph<'db>(
    db: &'db dyn Db,
    entry: ModuleId<'db>,
    def: DefId<'db>,
) -> Option<ModuleId<'db>> {
    let file = def.file(db);
    reachable_modules(db, entry)
        .into_iter()
        .find(|module| db.module_file(*module) == Some(file))
}

pub(crate) fn module_for_def_via_tree<'db>(
    db: &'db dyn Db,
    def: DefId<'db>,
) -> Option<ModuleId<'db>> {
    let path = hir::url_to_file_path(def.file(db).url(db))?;
    let tree = db.module_tree();
    let candidates = std::iter::once((LibraryId::Main, tree.main_root(db).clone()))
        .chain(std::iter::once((LibraryId::Std, tree.std_root(db).clone())))
        .chain(
            tree.external_roots(db)
                .iter()
                .map(|(name, root)| (LibraryId::External(name.clone()), root.clone())),
        );
    for (library, root) in candidates {
        if let Some(key) = module_key_for_path(library, &root, &path) {
            return Some(module_id_from_key(db, &key));
        }
    }
    None
}

pub(crate) fn is_canonical_std_def_named(db: &dyn Db, def: DefId<'_>, name: &str) -> bool {
    if def.name(db).as_deref() != Some(name) {
        return false;
    }

    let file = def.file(db);
    let tree = db.module_tree();
    let is_std_root_file = hir::url_to_file_path(file.url(db))
        .and_then(|path| module_key_for_path(LibraryId::Std, tree.std_root(db), &path))
        .is_some_and(|key| key.logical_path.as_slice() == ["std"]);

    is_std_root_file
        || module_id_for_source_file(db, file).is_some_and(|module| {
            module.library(db) == &LibraryId::Std && module.logical_path(db).as_slice() == ["std"]
        })
}
