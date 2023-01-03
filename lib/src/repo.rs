// Copyright 2020 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

use itertools::Itertools;
use once_cell::sync::OnceCell;
use thiserror::Error;

use self::dirty_cell::DirtyCell;
use crate::backend::{Backend, BackendError, BackendResult, ChangeId, CommitId, ObjectId, TreeId};
use crate::commit::Commit;
use crate::commit_builder::CommitBuilder;
use crate::dag_walk::topo_order_reverse;
use crate::git_backend::GitBackend;
use crate::index::{IndexRef, MutableIndex, ReadonlyIndex};
use crate::index_store::IndexStore;
use crate::local_backend::LocalBackend;
use crate::op_heads_store::{LockedOpHeads, OpHeads, OpHeadsStore};
use crate::op_store::{
    BranchTarget, OpStore, OperationId, OperationMetadata, RefTarget, WorkspaceId,
};
use crate::operation::Operation;
use crate::rewrite::DescendantRebaser;
use crate::settings::{RepoSettings, UserSettings};
use crate::simple_op_heads_store::SimpleOpHeadsStore;
use crate::simple_op_store::SimpleOpStore;
use crate::store::Store;
use crate::transaction::Transaction;
use crate::view::{RefName, View};
use crate::{backend, op_store};

// TODO: Should we implement From<&ReadonlyRepo> and From<&MutableRepo> for
// RepoRef?
#[derive(Clone, Copy)]
pub enum RepoRef<'a> {
    Readonly(&'a ReadonlyRepo),
    Mutable(&'a MutableRepo),
}

impl<'a> RepoRef<'a> {
    pub fn base_repo(&self) -> &ReadonlyRepo {
        match self {
            RepoRef::Readonly(repo) => repo,
            RepoRef::Mutable(repo) => repo.base_repo.as_ref(),
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        match self {
            RepoRef::Readonly(repo) => repo.store(),
            RepoRef::Mutable(repo) => repo.store(),
        }
    }

    pub fn op_store(&self) -> &Arc<dyn OpStore> {
        match self {
            RepoRef::Readonly(repo) => repo.op_store(),
            RepoRef::Mutable(repo) => repo.op_store(),
        }
    }

    pub fn index(&self) -> IndexRef<'a> {
        match self {
            RepoRef::Readonly(repo) => IndexRef::Readonly(repo.index()),
            RepoRef::Mutable(repo) => IndexRef::Mutable(repo.index()),
        }
    }

    pub fn view(&self) -> &View {
        match self {
            RepoRef::Readonly(repo) => repo.view(),
            RepoRef::Mutable(repo) => repo.view(),
        }
    }
}

pub struct ReadonlyRepo {
    repo_path: PathBuf,
    store: Arc<Store>,
    op_store: Arc<dyn OpStore>,
    op_heads_store: Arc<dyn OpHeadsStore>,
    operation: Operation,
    settings: RepoSettings,
    index_store: Arc<IndexStore>,
    index: OnceCell<Arc<ReadonlyIndex>>,
    view: View,
}

impl Debug for ReadonlyRepo {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Repo")
            .field("repo_path", &self.repo_path)
            .field("store", &self.store)
            .finish()
    }
}

impl ReadonlyRepo {
    pub fn default_op_store_factory() -> impl FnOnce(&Path) -> Box<dyn OpStore> {
        |store_path| Box::new(SimpleOpStore::init(store_path))
    }

    pub fn default_op_heads_store_factory() -> impl FnOnce(
        &Path,
        &Arc<dyn OpStore>,
        &op_store::View,
        OperationMetadata,
    ) -> (Box<dyn OpHeadsStore>, Operation) {
        |store_path, op_store, view, operation_metadata| {
            let (store, op) =
                SimpleOpHeadsStore::init(store_path, op_store, view, operation_metadata);
            (Box::new(store), op)
        }
    }

    pub fn init(
        user_settings: &UserSettings,
        repo_path: &Path,
        backend_factory: impl FnOnce(&Path) -> Box<dyn Backend>,
        op_store_factory: impl FnOnce(&Path) -> Box<dyn OpStore>,
        op_heads_store_factory: impl FnOnce(
            &Path,
            &Arc<dyn OpStore>,
            &op_store::View,
            OperationMetadata,
        ) -> (Box<dyn OpHeadsStore>, Operation),
    ) -> Result<Arc<ReadonlyRepo>, PathError> {
        let repo_path = repo_path.canonicalize().context(repo_path)?;

        let store_path = repo_path.join("store");
        fs::create_dir(&store_path).context(&store_path)?;
        let backend = backend_factory(&store_path);
        let backend_path = store_path.join("backend");
        fs::write(&backend_path, backend.name()).context(&backend_path)?;
        let store = Store::new(backend);
        let repo_settings = user_settings.with_repo(&repo_path).unwrap();

        let op_store_path = repo_path.join("op_store");
        fs::create_dir(&op_store_path).context(&op_store_path)?;
        let op_store = op_store_factory(&op_store_path);
        let op_store_type_path = op_store_path.join("type");
        fs::write(&op_store_type_path, op_store.name()).context(&op_store_type_path)?;
        let op_store = Arc::from(op_store);

        let mut root_view = op_store::View::default();
        root_view.head_ids.insert(store.root_commit_id().clone());
        root_view
            .public_head_ids
            .insert(store.root_commit_id().clone());

        let op_heads_path = repo_path.join("op_heads");
        fs::create_dir(&op_heads_path).context(&op_heads_path)?;
        let operation_metadata =
            crate::transaction::create_op_metadata(user_settings, "initialize repo".to_string());
        let (op_heads_store, init_op) =
            op_heads_store_factory(&op_heads_path, &op_store, &root_view, operation_metadata);
        let op_heads_type_path = op_heads_path.join("type");
        fs::write(&op_heads_type_path, op_heads_store.name()).context(&op_heads_type_path)?;
        let op_heads_store = Arc::from(op_heads_store);

        let index_path = repo_path.join("index");
        fs::create_dir(&index_path).context(&index_path)?;
        let index_store = Arc::new(IndexStore::init(index_path));

        let view = View::new(root_view);
        Ok(Arc::new(ReadonlyRepo {
            repo_path,
            store,
            op_store,
            op_heads_store,
            operation: init_op,
            settings: repo_settings,
            index_store,
            index: OnceCell::new(),
            view,
        }))
    }

    pub fn load_at_head(
        user_settings: &UserSettings,
        repo_path: &Path,
        store_factories: &StoreFactories,
    ) -> Result<Arc<ReadonlyRepo>, BackendError> {
        RepoLoader::init(user_settings, repo_path, store_factories)
            .load_at_head()
            .resolve(user_settings)
    }

    pub fn loader(&self) -> RepoLoader {
        RepoLoader {
            repo_path: self.repo_path.clone(),
            repo_settings: self.settings.clone(),
            store: self.store.clone(),
            op_store: self.op_store.clone(),
            op_heads_store: self.op_heads_store.clone(),
            index_store: self.index_store.clone(),
        }
    }

    pub fn as_repo_ref(&self) -> RepoRef {
        RepoRef::Readonly(self)
    }

    pub fn repo_path(&self) -> &PathBuf {
        &self.repo_path
    }

    pub fn op_id(&self) -> &OperationId {
        self.operation.id()
    }

    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn index(&self) -> &Arc<ReadonlyIndex> {
        self.index.get_or_init(|| {
            self.index_store
                .get_index_at_op(&self.operation, &self.store)
        })
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn op_store(&self) -> &Arc<dyn OpStore> {
        &self.op_store
    }

    pub fn op_heads_store(&self) -> &Arc<dyn OpHeadsStore> {
        &self.op_heads_store
    }

    pub fn index_store(&self) -> &Arc<IndexStore> {
        &self.index_store
    }

    pub fn settings(&self) -> &RepoSettings {
        &self.settings
    }

    pub fn start_transaction(
        self: &Arc<ReadonlyRepo>,
        user_settings: &UserSettings,
        description: &str,
    ) -> Transaction {
        let mut_repo = MutableRepo::new(self.clone(), self.index().clone(), &self.view);
        Transaction::new(mut_repo, user_settings, description)
    }

    pub fn reload_at_head(
        &self,
        user_settings: &UserSettings,
    ) -> Result<Arc<ReadonlyRepo>, BackendError> {
        self.loader().load_at_head().resolve(user_settings)
    }

    pub fn reload_at(&self, operation: &Operation) -> Arc<ReadonlyRepo> {
        self.loader().load_at(operation)
    }
}

pub enum RepoAtHead {
    Single(Arc<ReadonlyRepo>),
    Unresolved(Box<UnresolvedHeadRepo>),
}

impl RepoAtHead {
    pub fn resolve(self, user_settings: &UserSettings) -> Result<Arc<ReadonlyRepo>, BackendError> {
        match self {
            RepoAtHead::Single(repo) => Ok(repo),
            RepoAtHead::Unresolved(unresolved) => unresolved.resolve(user_settings),
        }
    }
}

pub struct UnresolvedHeadRepo {
    pub repo_loader: RepoLoader,
    pub locked_op_heads: LockedOpHeads,
    pub op_heads: Vec<Operation>,
}

impl UnresolvedHeadRepo {
    pub fn resolve(self, user_settings: &UserSettings) -> Result<Arc<ReadonlyRepo>, BackendError> {
        let base_repo = self.repo_loader.load_at(&self.op_heads[0]);
        let mut tx = base_repo.start_transaction(user_settings, "resolve concurrent operations");
        for other_op_head in self.op_heads.into_iter().skip(1) {
            tx.merge_operation(other_op_head);
            tx.mut_repo().rebase_descendants(user_settings)?;
        }
        let merged_repo = tx.write().leave_unpublished();
        self.locked_op_heads.finish(merged_repo.operation());
        Ok(merged_repo)
    }
}

type BackendFactory = Box<dyn Fn(&Path) -> Box<dyn Backend>>;
type OpStoreFactory = Box<dyn Fn(&Path) -> Box<dyn OpStore>>;
type OpHeadsStoreFactory = Box<dyn Fn(&Path) -> Box<dyn OpHeadsStore>>;

pub struct StoreFactories {
    backend_factories: HashMap<String, BackendFactory>,
    op_store_factories: HashMap<String, OpStoreFactory>,
    op_heads_store_factories: HashMap<String, OpHeadsStoreFactory>,
}

impl Default for StoreFactories {
    fn default() -> Self {
        let mut factories = StoreFactories::empty();

        // Backends
        factories.add_backend(
            "local",
            Box::new(|store_path| Box::new(LocalBackend::load(store_path))),
        );
        factories.add_backend(
            "git",
            Box::new(|store_path| Box::new(GitBackend::load(store_path))),
        );

        // OpStores
        factories.add_op_store(
            "simple_op_store",
            Box::new(|store_path| Box::new(SimpleOpStore::load(store_path))),
        );

        // OpHeadsStores
        factories.add_op_heads_store(
            "simple_op_heads_store",
            Box::new(|store_path| Box::new(SimpleOpHeadsStore::load(store_path))),
        );

        factories
    }
}

impl StoreFactories {
    pub fn empty() -> Self {
        StoreFactories {
            backend_factories: HashMap::new(),
            op_store_factories: HashMap::new(),
            op_heads_store_factories: HashMap::new(),
        }
    }

    pub fn add_backend(&mut self, name: &str, factory: BackendFactory) {
        self.backend_factories.insert(name.to_string(), factory);
    }

    pub fn load_backend(&self, store_path: &Path) -> Box<dyn Backend> {
        // TODO: Change the 'backend' file to 'type', for consistency with other stores
        let backend_type = match fs::read_to_string(store_path.join("backend")) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // For compatibility with existing repos. TODO: Delete in spring of 2023 or so.
                let inferred_type = if store_path.join("git_target").is_file() {
                    String::from("git")
                } else {
                    String::from("local")
                };
                fs::write(store_path.join("backend"), &inferred_type).unwrap();
                inferred_type
            }
            Err(_) => {
                panic!("Failed to read backend type");
            }
        };
        let backend_factory = self
            .backend_factories
            .get(&backend_type)
            .expect("Unexpected backend type");
        backend_factory(store_path)
    }

    pub fn add_op_store(&mut self, name: &str, factory: OpStoreFactory) {
        self.op_store_factories.insert(name.to_string(), factory);
    }

    pub fn load_op_store(&self, store_path: &Path) -> Box<dyn OpStore> {
        let op_store_type = match fs::read_to_string(store_path.join("type")) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // For compatibility with existing repos. TODO: Delete in 0.8+
                let default_type = String::from("simple_op_store");
                fs::write(store_path.join("type"), &default_type).unwrap();
                default_type
            }
            Err(_) => {
                panic!("Failed to read op_store type");
            }
        };
        let op_store_factory = self
            .op_store_factories
            .get(&op_store_type)
            .expect("Unexpected op_store type");
        op_store_factory(store_path)
    }

    pub fn add_op_heads_store(&mut self, name: &str, factory: OpHeadsStoreFactory) {
        self.op_heads_store_factories
            .insert(name.to_string(), factory);
    }

    pub fn load_op_heads_store(&self, store_path: &Path) -> Box<dyn OpHeadsStore> {
        let op_heads_store_type = match fs::read_to_string(store_path.join("type")) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // For compatibility with existing repos. TODO: Delete in 0.8+
                let default_type = String::from("simple_op_heads_store");
                fs::write(store_path.join("type"), &default_type).unwrap();
                default_type
            }
            Err(_) => {
                panic!("Failed to read op_heads_store type");
            }
        };
        let op_heads_store_factory = self
            .op_heads_store_factories
            .get(&op_heads_store_type)
            .expect("Unexpected op_heads_store type");
        op_heads_store_factory(store_path)
    }
}

#[derive(Clone)]
pub struct RepoLoader {
    repo_path: PathBuf,
    repo_settings: RepoSettings,
    store: Arc<Store>,
    op_store: Arc<dyn OpStore>,
    op_heads_store: Arc<dyn OpHeadsStore>,
    index_store: Arc<IndexStore>,
}

impl RepoLoader {
    pub fn init(
        user_settings: &UserSettings,
        repo_path: &Path,
        store_factories: &StoreFactories,
    ) -> Self {
        let store = Store::new(store_factories.load_backend(&repo_path.join("store")));
        let repo_settings = user_settings.with_repo(repo_path).unwrap();
        let op_store = Arc::from(store_factories.load_op_store(&repo_path.join("op_store")));
        let op_heads_store =
            Arc::from(store_factories.load_op_heads_store(&repo_path.join("op_heads")));
        let index_store = Arc::new(IndexStore::load(repo_path.join("index")));
        Self {
            repo_path: repo_path.to_path_buf(),
            repo_settings,
            store,
            op_store,
            op_heads_store,
            index_store,
        }
    }

    pub fn repo_path(&self) -> &PathBuf {
        &self.repo_path
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn index_store(&self) -> &Arc<IndexStore> {
        &self.index_store
    }

    pub fn op_store(&self) -> &Arc<dyn OpStore> {
        &self.op_store
    }

    pub fn op_heads_store(&self) -> &Arc<dyn OpHeadsStore> {
        &self.op_heads_store
    }

    pub fn load_at_head(&self) -> RepoAtHead {
        let op_heads = self.op_heads_store.get_heads(&self.op_store).unwrap();
        match op_heads {
            OpHeads::Single(op) => {
                let view = View::new(op.view().take_store_view());
                RepoAtHead::Single(self._finish_load(op, view))
            }
            OpHeads::Unresolved {
                locked_op_heads,
                op_heads,
            } => RepoAtHead::Unresolved(Box::new(UnresolvedHeadRepo {
                repo_loader: self.clone(),
                locked_op_heads,
                op_heads,
            })),
        }
    }

    pub fn load_at(&self, op: &Operation) -> Arc<ReadonlyRepo> {
        let view = View::new(op.view().take_store_view());
        self._finish_load(op.clone(), view)
    }

    pub fn create_from(
        &self,
        operation: Operation,
        view: View,
        index: Arc<ReadonlyIndex>,
    ) -> Arc<ReadonlyRepo> {
        let repo = ReadonlyRepo {
            repo_path: self.repo_path.clone(),
            store: self.store.clone(),
            op_store: self.op_store.clone(),
            op_heads_store: self.op_heads_store.clone(),
            operation,
            settings: self.repo_settings.clone(),
            index_store: self.index_store.clone(),
            index: OnceCell::with_value(index),
            view,
        };
        Arc::new(repo)
    }

    fn _finish_load(&self, operation: Operation, view: View) -> Arc<ReadonlyRepo> {
        let repo = ReadonlyRepo {
            repo_path: self.repo_path.clone(),
            store: self.store.clone(),
            op_store: self.op_store.clone(),
            op_heads_store: self.op_heads_store.clone(),
            operation,
            settings: self.repo_settings.clone(),
            index_store: self.index_store.clone(),
            index: OnceCell::new(),
            view,
        };
        Arc::new(repo)
    }
}

pub struct MutableRepo {
    base_repo: Arc<ReadonlyRepo>,
    index: MutableIndex,
    view: DirtyCell<View>,
    rewritten_commits: HashMap<CommitId, HashSet<CommitId>>,
    abandoned_commits: HashSet<CommitId>,
}

impl MutableRepo {
    pub fn new(
        base_repo: Arc<ReadonlyRepo>,
        index: Arc<ReadonlyIndex>,
        view: &View,
    ) -> MutableRepo {
        let mut_view = view.clone();
        let mut_index = MutableIndex::incremental(index);
        MutableRepo {
            base_repo,
            index: mut_index,
            view: DirtyCell::with_clean(mut_view),
            rewritten_commits: Default::default(),
            abandoned_commits: Default::default(),
        }
    }

    pub fn as_repo_ref(&self) -> RepoRef {
        RepoRef::Mutable(self)
    }

    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        &self.base_repo
    }

    pub fn store(&self) -> &Arc<Store> {
        self.base_repo.store()
    }

    pub fn op_store(&self) -> &Arc<dyn OpStore> {
        self.base_repo.op_store()
    }

    pub fn index(&self) -> &MutableIndex {
        &self.index
    }

    pub fn view(&self) -> &View {
        self.view
            .get_or_ensure_clean(|v| self.enforce_view_invariants(v))
    }

    fn view_mut(&mut self) -> &mut View {
        self.view.get_mut()
    }

    pub fn has_changes(&self) -> bool {
        self.view() != &self.base_repo.view
    }

    pub fn consume(self) -> (MutableIndex, View) {
        self.view.ensure_clean(|v| self.enforce_view_invariants(v));
        (self.index, self.view.into_inner())
    }

    pub fn new_commit(
        &mut self,
        settings: &UserSettings,
        parents: Vec<CommitId>,
        tree_id: TreeId,
    ) -> CommitBuilder {
        CommitBuilder::for_new_commit(self, settings, parents, tree_id)
    }

    pub fn rewrite_commit(
        &mut self,
        settings: &UserSettings,
        predecessor: &Commit,
    ) -> CommitBuilder {
        CommitBuilder::for_rewrite_from(self, settings, predecessor)
    }

    pub fn write_commit(&mut self, commit: backend::Commit) -> BackendResult<Commit> {
        let commit = self.store().write_commit(commit)?;
        self.add_head(&commit);
        Ok(commit)
    }

    /// Record a commit as having been rewritten in this transaction. This
    /// record is used by `rebase_descendants()`.
    ///
    /// Rewritten commits don't have to be recorded here. This is just a
    /// convenient place to record it. It won't matter after the transaction
    /// has been committed.
    pub fn record_rewritten_commit(&mut self, old_id: CommitId, new_id: CommitId) {
        assert_ne!(old_id, *self.store().root_commit_id());
        self.rewritten_commits
            .entry(old_id)
            .or_default()
            .insert(new_id);
    }

    pub fn clear_rewritten_commits(&mut self) {
        self.rewritten_commits.clear();
    }

    /// Record a commit as having been abandoned in this transaction. This
    /// record is used by `rebase_descendants()`.
    ///
    /// Abandoned commits don't have to be recorded here. This is just a
    /// convenient place to record it. It won't matter after the transaction
    /// has been committed.
    pub fn record_abandoned_commit(&mut self, old_id: CommitId) {
        assert_ne!(old_id, *self.store().root_commit_id());
        self.abandoned_commits.insert(old_id);
    }

    pub fn clear_abandoned_commits(&mut self) {
        self.abandoned_commits.clear();
    }

    pub fn has_rewrites(&self) -> bool {
        !(self.rewritten_commits.is_empty() && self.abandoned_commits.is_empty())
    }

    /// Creates a `DescendantRebaser` to rebase descendants of the recorded
    /// rewritten and abandoned commits.
    pub fn create_descendant_rebaser<'settings, 'repo>(
        &'repo mut self,
        settings: &'settings UserSettings,
    ) -> DescendantRebaser<'settings, 'repo> {
        DescendantRebaser::new(
            settings,
            self,
            self.rewritten_commits.clone(),
            self.abandoned_commits.clone(),
        )
    }

    pub fn rebase_descendants(&mut self, settings: &UserSettings) -> Result<usize, BackendError> {
        if !self.has_rewrites() {
            // Optimization
            return Ok(0);
        }
        let mut rebaser = self.create_descendant_rebaser(settings);
        rebaser.rebase_all()?;
        Ok(rebaser.rebased().len())
    }

    pub fn set_wc_commit(
        &mut self,
        workspace_id: WorkspaceId,
        commit_id: CommitId,
    ) -> Result<(), RewriteRootCommit> {
        if &commit_id == self.store().root_commit_id() {
            return Err(RewriteRootCommit);
        }
        self.view_mut().set_wc_commit(workspace_id, commit_id);
        Ok(())
    }

    pub fn remove_wc_commit(&mut self, workspace_id: &WorkspaceId) {
        self.view_mut().remove_wc_commit(workspace_id);
    }

    pub fn check_out(
        &mut self,
        workspace_id: WorkspaceId,
        settings: &UserSettings,
        commit: &Commit,
    ) -> BackendResult<Commit> {
        self.leave_commit(&workspace_id);
        let wc_commit = self
            .new_commit(
                settings,
                vec![commit.id().clone()],
                commit.tree_id().clone(),
            )
            .write()?;
        self.set_wc_commit(workspace_id, wc_commit.id().clone())
            .unwrap();
        Ok(wc_commit)
    }

    pub fn edit(
        &mut self,
        workspace_id: WorkspaceId,
        commit: &Commit,
    ) -> Result<(), RewriteRootCommit> {
        self.leave_commit(&workspace_id);
        self.set_wc_commit(workspace_id, commit.id().clone())
    }

    fn leave_commit(&mut self, workspace_id: &WorkspaceId) {
        let maybe_wc_commit_id = self
            .view
            .with_ref(|v| v.get_wc_commit_id(workspace_id).cloned());
        if let Some(wc_commit_id) = maybe_wc_commit_id {
            let wc_commit = self.store().get_commit(&wc_commit_id).unwrap();
            if wc_commit.is_empty()
                && wc_commit.description().is_empty()
                && self.view().heads().contains(wc_commit.id())
            {
                // Abandon the checkout we're leaving if it's empty and a head commit
                self.record_abandoned_commit(wc_commit_id);
            }
        }
    }

    fn enforce_view_invariants(&self, view: &mut View) {
        let view = view.store_view_mut();
        view.public_head_ids = self
            .index
            .heads(view.public_head_ids.iter())
            .iter()
            .cloned()
            .collect();
        view.head_ids.extend(view.public_head_ids.iter().cloned());
        view.head_ids = self
            .index
            .heads(view.head_ids.iter())
            .iter()
            .cloned()
            .collect();
    }

    pub fn add_head(&mut self, head: &Commit) {
        let current_heads = self.view.get_mut().heads();
        // Use incremental update for common case of adding a single commit on top a
        // current head. TODO: Also use incremental update when adding a single
        // commit on top a non-head.
        if head
            .parent_ids()
            .iter()
            .all(|parent_id| current_heads.contains(parent_id))
        {
            self.index.add_commit(head);
            self.view.get_mut().add_head(head.id());
            for parent_id in head.parent_ids() {
                self.view.get_mut().remove_head(parent_id);
            }
        } else {
            let missing_commits = topo_order_reverse(
                vec![head.clone()],
                Box::new(|commit: &Commit| commit.id().clone()),
                Box::new(|commit: &Commit| -> Vec<Commit> {
                    commit
                        .parents()
                        .into_iter()
                        .filter(|parent| !self.index.has_id(parent.id()))
                        .collect()
                }),
            );
            for missing_commit in missing_commits.iter().rev() {
                self.index.add_commit(missing_commit);
            }
            self.view.get_mut().add_head(head.id());
            self.view.mark_dirty();
        }
    }

    pub fn remove_head(&mut self, head: &CommitId) {
        self.view_mut().remove_head(head);
        self.view.mark_dirty();
    }

    pub fn add_public_head(&mut self, head: &Commit) {
        self.view_mut().add_public_head(head.id());
        self.view.mark_dirty();
    }

    pub fn remove_public_head(&mut self, head: &CommitId) {
        self.view_mut().remove_public_head(head);
        self.view.mark_dirty();
    }

    pub fn get_branch(&self, name: &str) -> Option<BranchTarget> {
        self.view.with_ref(|v| v.get_branch(name).cloned())
    }

    pub fn set_branch(&mut self, name: String, target: BranchTarget) {
        self.view_mut().set_branch(name, target);
    }

    pub fn remove_branch(&mut self, name: &str) {
        self.view_mut().remove_branch(name);
    }

    pub fn get_local_branch(&self, name: &str) -> Option<RefTarget> {
        self.view.with_ref(|v| v.get_local_branch(name))
    }

    pub fn set_local_branch(&mut self, name: String, target: RefTarget) {
        self.view_mut().set_local_branch(name, target);
    }

    pub fn remove_local_branch(&mut self, name: &str) {
        self.view_mut().remove_local_branch(name);
    }

    pub fn get_remote_branch(&self, name: &str, remote_name: &str) -> Option<RefTarget> {
        self.view
            .with_ref(|v| v.get_remote_branch(name, remote_name))
    }

    pub fn set_remote_branch(&mut self, name: String, remote_name: String, target: RefTarget) {
        self.view_mut().set_remote_branch(name, remote_name, target);
    }

    pub fn remove_remote_branch(&mut self, name: &str, remote_name: &str) {
        self.view_mut().remove_remote_branch(name, remote_name);
    }

    pub fn rename_remote(&mut self, old: &str, new: &str) {
        self.view_mut().rename_remote(old, new);
    }

    pub fn get_tag(&self, name: &str) -> Option<RefTarget> {
        self.view.with_ref(|v| v.get_tag(name))
    }

    pub fn set_tag(&mut self, name: String, target: RefTarget) {
        self.view_mut().set_tag(name, target);
    }

    pub fn remove_tag(&mut self, name: &str) {
        self.view_mut().remove_tag(name);
    }

    pub fn get_git_ref(&self, name: &str) -> Option<RefTarget> {
        self.view.with_ref(|v| v.get_git_ref(name))
    }

    pub fn set_git_ref(&mut self, name: String, target: RefTarget) {
        self.view_mut().set_git_ref(name, target);
    }

    pub fn remove_git_ref(&mut self, name: &str) {
        self.view_mut().remove_git_ref(name);
    }

    pub fn set_git_head(&mut self, head_id: CommitId) {
        self.view_mut().set_git_head(head_id);
    }

    pub fn clear_git_head(&mut self) {
        self.view_mut().clear_git_head();
    }

    pub fn set_view(&mut self, data: op_store::View) {
        self.view_mut().set_view(data);
        self.view.mark_dirty();
    }

    pub fn merge(&mut self, base_repo: &ReadonlyRepo, other_repo: &ReadonlyRepo) {
        // First, merge the index, so we can take advantage of a valid index when
        // merging the view. Merging in base_repo's index isn't typically
        // necessary, but it can be if base_repo is ahead of either self or other_repo
        // (e.g. because we're undoing an operation that hasn't been published).
        self.index.merge_in(base_repo.index());
        self.index.merge_in(other_repo.index());

        self.view.ensure_clean(|v| self.enforce_view_invariants(v));
        self.merge_view(&base_repo.view, &other_repo.view);
        self.view.mark_dirty();
    }

    fn merge_view(&mut self, base: &View, other: &View) {
        // Merge checkouts. If there's a conflict, we keep the self side.
        for (workspace_id, base_checkout) in base.wc_commit_ids() {
            let self_checkout = self.view().get_wc_commit_id(workspace_id);
            let other_checkout = other.get_wc_commit_id(workspace_id);
            if other_checkout == Some(base_checkout) || other_checkout == self_checkout {
                // The other side didn't change or both sides changed in the
                // same way.
            } else if let Some(other_checkout) = other_checkout {
                if self_checkout == Some(base_checkout) {
                    self.view_mut()
                        .set_wc_commit(workspace_id.clone(), other_checkout.clone());
                }
            } else {
                // The other side removed the workspace. We want to remove it even if the self
                // side changed the checkout.
                self.view_mut().remove_wc_commit(workspace_id);
            }
        }
        for (workspace_id, other_checkout) in other.wc_commit_ids() {
            if self.view().get_wc_commit_id(workspace_id).is_none()
                && base.get_wc_commit_id(workspace_id).is_none()
            {
                // The other side added the workspace.
                self.view_mut()
                    .set_wc_commit(workspace_id.clone(), other_checkout.clone());
            }
        }

        for removed_head in base.public_heads().difference(other.public_heads()) {
            self.view_mut().remove_public_head(removed_head);
        }
        for added_head in other.public_heads().difference(base.public_heads()) {
            self.view_mut().add_public_head(added_head);
        }

        let base_heads = base.heads().iter().cloned().collect_vec();
        let own_heads = self.view().heads().iter().cloned().collect_vec();
        let other_heads = other.heads().iter().cloned().collect_vec();
        self.record_rewrites(&base_heads, &own_heads);
        self.record_rewrites(&base_heads, &other_heads);
        // No need to remove heads removed by `other` because we already marked them
        // abandoned or rewritten.
        for added_head in other.heads().difference(base.heads()) {
            self.view_mut().add_head(added_head);
        }

        let mut maybe_changed_ref_names = HashSet::new();

        let base_branches: HashSet<_> = base.branches().keys().cloned().collect();
        let other_branches: HashSet<_> = other.branches().keys().cloned().collect();
        for branch_name in base_branches.union(&other_branches) {
            let base_branch = base.branches().get(branch_name);
            let other_branch = other.branches().get(branch_name);
            if other_branch == base_branch {
                // Unchanged on other side
                continue;
            }

            maybe_changed_ref_names.insert(RefName::LocalBranch(branch_name.clone()));
            if let Some(branch) = base_branch {
                for remote in branch.remote_targets.keys() {
                    maybe_changed_ref_names.insert(RefName::RemoteBranch {
                        branch: branch_name.clone(),
                        remote: remote.clone(),
                    });
                }
            }
            if let Some(branch) = other_branch {
                for remote in branch.remote_targets.keys() {
                    maybe_changed_ref_names.insert(RefName::RemoteBranch {
                        branch: branch_name.clone(),
                        remote: remote.clone(),
                    });
                }
            }
        }

        for tag_name in base.tags().keys() {
            maybe_changed_ref_names.insert(RefName::Tag(tag_name.clone()));
        }
        for tag_name in other.tags().keys() {
            maybe_changed_ref_names.insert(RefName::Tag(tag_name.clone()));
        }

        for git_ref_name in base.git_refs().keys() {
            maybe_changed_ref_names.insert(RefName::GitRef(git_ref_name.clone()));
        }
        for git_ref_name in other.git_refs().keys() {
            maybe_changed_ref_names.insert(RefName::GitRef(git_ref_name.clone()));
        }

        for ref_name in maybe_changed_ref_names {
            let base_target = base.get_ref(&ref_name);
            let other_target = other.get_ref(&ref_name);
            self.view.get_mut().merge_single_ref(
                self.index.as_index_ref(),
                &ref_name,
                base_target.as_ref(),
                other_target.as_ref(),
            );
        }
    }

    /// Finds and records commits that were rewritten or abandoned between
    /// `old_heads` and `new_heads`.
    fn record_rewrites(&mut self, old_heads: &[CommitId], new_heads: &[CommitId]) {
        let mut removed_changes: HashMap<ChangeId, Vec<CommitId>> = HashMap::new();
        for removed in self.index.walk_revs(old_heads, new_heads) {
            removed_changes
                .entry(removed.change_id())
                .or_default()
                .push(removed.commit_id());
        }
        if removed_changes.is_empty() {
            return;
        }

        let mut rewritten_changes = HashSet::new();
        let mut rewritten_commits: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        for added in self.index.walk_revs(new_heads, old_heads) {
            let change_id = added.change_id();
            if let Some(old_commits) = removed_changes.get(&change_id) {
                for old_commit in old_commits {
                    rewritten_commits
                        .entry(old_commit.clone())
                        .or_default()
                        .push(added.commit_id());
                }
            }
            rewritten_changes.insert(change_id);
        }
        for (old_commit, new_commits) in rewritten_commits {
            for new_commit in new_commits {
                self.record_rewritten_commit(old_commit.clone(), new_commit);
            }
        }

        for (change_id, removed_commit_ids) in &removed_changes {
            if !rewritten_changes.contains(change_id) {
                for removed_commit_id in removed_commit_ids {
                    self.record_abandoned_commit(removed_commit_id.clone());
                }
            }
        }
    }

    pub fn merge_single_ref(
        &mut self,
        ref_name: &RefName,
        base_target: Option<&RefTarget>,
        other_target: Option<&RefTarget>,
    ) {
        self.view.get_mut().merge_single_ref(
            self.index.as_index_ref(),
            ref_name,
            base_target,
            other_target,
        );
    }
}

/// Error from attempts to check out the root commit for editing
#[derive(Debug, Copy, Clone, Error)]
#[error("Cannot rewrite the root commit")]
pub struct RewriteRootCommit;

#[derive(Debug, Error)]
#[error("Cannot access {path}")]
pub struct PathError {
    pub path: PathBuf,
    #[source]
    pub error: io::Error,
}

pub(crate) trait IoResultExt<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError> {
        self.map_err(|error| PathError {
            path: path.as_ref().to_path_buf(),
            error,
        })
    }
}

mod dirty_cell {
    use std::cell::{Cell, RefCell};

    /// Cell that lazily updates the value after `mark_dirty()`.
    #[derive(Clone, Debug)]
    pub struct DirtyCell<T> {
        value: RefCell<T>,
        dirty: Cell<bool>,
    }

    impl<T> DirtyCell<T> {
        pub fn with_clean(value: T) -> Self {
            DirtyCell {
                value: RefCell::new(value),
                dirty: Cell::new(false),
            }
        }

        pub fn get_or_ensure_clean(&self, f: impl FnOnce(&mut T)) -> &T {
            // SAFETY: get_mut/mark_dirty(&mut self) should invalidate any previously-clean
            // references leaked by this method. Clean value never changes until then.
            self.ensure_clean(f);
            unsafe { &*self.value.as_ptr() }
        }

        pub fn ensure_clean(&self, f: impl FnOnce(&mut T)) {
            if self.dirty.get() {
                // This borrow_mut() ensures that there is no dirty temporary reference.
                // Panics if ensure_clean() is invoked from with_ref() callback for example.
                f(&mut self.value.borrow_mut());
                self.dirty.set(false);
            }
        }

        pub fn into_inner(self) -> T {
            self.value.into_inner()
        }

        pub fn with_ref<R>(&self, f: impl FnOnce(&T) -> R) -> R {
            f(&self.value.borrow())
        }

        pub fn get_mut(&mut self) -> &mut T {
            self.value.get_mut()
        }

        pub fn mark_dirty(&mut self) {
            *self.dirty.get_mut() = true;
        }
    }
}

/// Conceptually like `HashMap<Vec<I>, V>`, but supports lookup by prefixes
/// of keys.
#[derive(Debug, Clone)]
pub struct Trie<I: Eq + Hash + Clone, V> {
    // TODO: The trie currently uses more memory (~4x by one measurement) than
    // a simple HashSet of commit & change ids would. This could be addressed by:
    //
    // 2. Having better supposer for iterating over keys, thus avoiding the
    // need to store the key as part of the value in many applications. Note
    // that this may require allocating objects (e.g. a deque) to store the
    // complete keys.
    //
    // It's unclear if either of these is worth the complexity.
    key_prefix: Vec<I>,
    // TODO: This is fun an all, but the code may be easier to understand if we have *either*
    // key_prefix *or* next_level. That only makes sense for tries of hashes (where only the tails
    // become key_prefixes)
    value: Option<V>,
    next_level: HashMap<I, Box<Trie<I, V>>>,
}

impl<I: Eq + Hash + Clone, V> Default for Trie<I, V> {
    fn default() -> Self {
        Trie::new()
    }
}

impl<I: Eq + Hash + Clone, V> Trie<I, V> {
    pub fn new() -> Self {
        Self {
            key_prefix: vec![],
            value: None,
            next_level: HashMap::new(),
        }
    }

    fn len_common_prefix(left: &[I], right: &[I]) -> usize {
        let mut result = 0;
        for (a, b) in std::iter::zip(left, right) {
            if a != b {
                break;
            }
            result += 1;
        }
        result
    }

    pub fn insert(&mut self, key: &[I], value: V) -> bool {
        if self.key_prefix.is_empty() && self.value.is_none() && self.next_level.is_empty() {
            self.value = Some(value);
            self.key_prefix = key.to_vec();
            true
        } else if key.starts_with(&self.key_prefix) {
            match &key[self.key_prefix.len()..] {
                [] => {
                    let return_value = self.value.is_none();
                    self.value = Some(value);
                    return_value
                }
                [next_char, rest @ ..] => self
                    .next_level
                    .entry(next_char.clone())
                    .or_default()
                    .insert(rest, value),
            }
        } else {
            let common = Self::len_common_prefix(key, &self.key_prefix);
            let new_trie = Box::new(Self {
                key_prefix: self.key_prefix[common + 1..].to_vec(),
                value: self.value.take(),
                next_level: self.next_level.drain().collect(),
            });
            self.next_level
                .insert(self.key_prefix[common].clone(), new_trie);
            self.key_prefix.truncate(common);
            // Now `key` starts with `self.key_prefix`. The trie is restructured but
            // equivalent to what it was before.

            self.insert(key, value)
        }
    }

    pub fn get<'a>(&'a self, key: &[I]) -> Option<&'a V> {
        if !key.starts_with(&self.key_prefix) {
            return None;
        }
        if let Some(next_char) = key.get(self.key_prefix.len()) {
            self.next_level
                .get(next_char)
                .and_then(|subtrie| subtrie.get(&key[self.key_prefix.len() + 1..]))
        } else {
            self.value.as_ref()
        }
    }

    pub fn itervalues(&self) -> TrieValueIterator<I, V> {
        TrieValueIterator::new(self)
    }

    /// This function returns the shortest length of a prefix of `key` that
    /// corresponds to a trie that is either a) empty or b) contains only a
    /// single element that matches `key` exactly.
    ///
    /// In the special case when there are keys in the trie for which our `key`
    /// is an exact prefix, returns `key.len() + 1`. Conceptually, in order to
    /// disambiguate, you need every letter of the key *and* the additional
    /// fact that it's the entire key). This case is extremely unlikely for
    /// hashes with 12+ hexadecimal characters.
    pub fn shortest_unique_prefix_len(&self, key: &[I]) -> usize {
        let common = Self::len_common_prefix(key, &self.key_prefix);
        let prefix_len = self.key_prefix.len();
        if common < prefix_len {
            return common + 1;
        }
        // self.key_prefix is a prefix of key
        match &key[prefix_len..] {
            [] => {
                if self.next_level.is_empty() {
                    0
                } else {
                    // The special case: there are keys in the trie for which the original
                    // `key` (from the first level of recursion) is a prefix.
                    key.len() + 1
                }
            }
            [first, rest @ ..] => {
                match self.next_level.get(first) {
                    None => {
                        // The key we're looking for is not in our trie. We may or may not need
                        // one more character (let's say `I` is `u8`
                        // to simplify terminology) to distinguish
                        // it from all the keys that *are* in our trie.
                        if self.next_level.is_empty() {
                            prefix_len // TODO: test, double-check. Is it always
                                       // +1?
                                       // Shouldn't we check that
                                       // self.value.is_some()?
                        } else {
                            prefix_len + 1
                        }
                    }
                    Some(next_trie) => {
                        match next_trie.shortest_unique_prefix_len(rest) {
                            0 => {
                                // The `next_trie` subtrie of our trie is either empty or contains a
                                // single element matching our key exactly.
                                if self.next_level.len() == 1 && self.value.is_none() {
                                    // Our trie has the same property. There's no need for more
                                    // characters to distinguish our key from all other keys.
                                    // TODO: Test the second `&&` branch
                                    0 // Shouldn't happen in the radix tree
                                } else {
                                    prefix_len + 1
                                }
                            }
                            n => prefix_len + n + 1,
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrieValueIterator<'a, I: Eq + Hash + Clone, V> {
    current_value: Option<&'a V>,
    subtrie_iter: Option<Box<TrieValueIterator<'a, I, V>>>, /* Iterating inside a value of
                                                             * `next_level_iter` */
    next_level_iter: std::collections::hash_map::Iter<'a, I, Box<Trie<I, V>>>,
}

impl<'a, I: Eq + Hash + Clone, V> TrieValueIterator<'a, I, V> {
    pub fn new(trie: &'a Trie<I, V>) -> Self {
        Self {
            current_value: trie.value.as_ref(),
            subtrie_iter: None,
            next_level_iter: trie.next_level.iter(),
        }
    }
}

impl<'a, I: Eq + Hash + Clone, V> Iterator for TrieValueIterator<'a, I, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<&'a V> {
        if let Some(value) = self.current_value {
            self.current_value = None;
            return Some(value);
        }

        if let Some(subtrie_iter) = self.subtrie_iter.as_mut() {
            if let Some(value) = subtrie_iter.next() {
                return Some(value);
            }
        }

        if let Some((_key, next_trie)) = self.next_level_iter.next() {
            self.subtrie_iter = Some(Box::new(TrieValueIterator::new(next_trie)));
            return self.next();
        }

        None
    }
}

#[test]
fn test_trie() {
    let mut trie = Trie::new();
    assert_eq!(trie.itervalues().next(), None);
    trie.insert(b"ab", "val1".to_string());
    trie.insert(b"acd", "val2".to_string());
    assert_eq!(trie.shortest_unique_prefix_len(b"acd"), 2);
    assert_eq!(trie.shortest_unique_prefix_len(b"ac"), 3);

    let mut trie = Trie::new();
    assert_eq!(trie.itervalues().next(), None);
    trie.insert(b"ab", "val1".to_string());
    trie.insert(b"acd", "val2".to_string());
    trie.insert(b"acf", "val2".to_string());
    trie.insert(b"a", "val3".to_string());
    trie.insert(b"ba", "val2".to_string());

    // In case further debugging is needed
    // println!("{trie:?}");
    // let mut iter = trie.itervalues();
    // println!("{:?}", iter.next());
    // println!("{:?}", iter);
    // println!("{:?}", iter.next());
    // println!("{:?}", iter);

    assert_eq!(trie.get(b"a"), Some(&"val3".to_string()));
    assert_eq!(trie.get(b"ab"), Some(&"val1".to_string()));
    assert_eq!(trie.get(b"b"), None);

    assert_eq!(trie.shortest_unique_prefix_len(b"a"), 2); // Unlikely for hashes case: the entire length of the key is an insufficient
                                                          // prefix
    assert_eq!(trie.shortest_unique_prefix_len(b"ba"), 1);
    assert_eq!(trie.shortest_unique_prefix_len(b"ab"), 2);
    assert_eq!(trie.shortest_unique_prefix_len(b"acd"), 3);
    // If it were there, the length would be 1.
    assert_eq!(trie.shortest_unique_prefix_len(b"c"), 1);

    let mut values = trie.itervalues().collect_vec();
    values.sort();
    assert_eq!(values, vec!["val1", "val2", "val2", "val2", "val3"])
}
