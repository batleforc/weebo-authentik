# Plan affiné - Authentik Kubernetes Operator

Suite de `.prompt/init.md`. Contexte glané depuis le module Terraform réel
(`batleforc/weebo-si:2.terra/auth/terra-map`, branche `develop`) pour ancrer
le scope dans du réel plutôt que du spéculatif.

## Inventaire du module Terraform à remplacer

- **Partagé / global**
  - `authentik_group` : hiérarchie `weebo_user -> weebo_moderator -> weebo_admin`,
    champ `is_superuser`, `parents`.
  - `authentik_flow` : un seul flow custom (device code). Le reste (auth,
    invalidation, recovery, unenrollment, user-settings) est du lookup de
    flows par défaut via data source.
  - `authentik_brand` : une seule brand custom "weebo", référence les flows
    par défaut + logo/favicon/background.
  - Data sources en lecture seule : flows par défaut, certificate key pair,
    property mappings de scope (email/profile/openid/offline_access), API
    access mapping, embedded outpost.
- **Par application, variante OAuth2 (argo, che-cluster, harbor, s3,
  vault — 5 des 6 apps)**
  - `authentik_provider_oauth2` (client_id, flows, signing_key,
    allowed_redirect_uris avec `matching_mode` strict/regex,
    property_mappings, grant_types=authorization_code)
  - `authentik_application` (name, slug, protocol_provider, meta_icon)
  - `authentik_policy_binding` (target=application, group, order) — c'est
    le mécanisme d'accès qui deviendra l'allow-list/RBAC applicatif.
  - Un secret Vault structuré est écrit à chaque fois :
    `AUTHENTIK_CLIENT_ID` / `AUTHENTIK_CLIENT_SECRET` / `AUTHENTIK_URL`.
    C'est la forme canonique que le port `SecretStore` doit produire.
- **Par application, variante Proxy (longhorn — la 6e app, initialement
  ratée en scannant juste `harbor.tf` comme "exemple type")**
  - `authentik_provider_proxy` (`internal_host`, `external_host`,
    `authorization_flow`, `invalidation_flow`) — pas de client_id/secret,
    l'auth se fait au niveau de l'outpost, donc **pas d'écriture
    `SecretStore`** pour ce type de provider.
  - `authentik_outpost_provider_attachment` : attache le provider proxy
    à `data.authentik_outpost.embedded` (l'outpost embarqué livré avec
    Authentik — toujours lookup, jamais créé). Aucun autre outpost
    utilisé dans le module.
  - `authentik_application` + `authentik_policy_binding` identiques au
    pattern OAuth2.
- **Identité utilisateur**
  - `authentik_user` (`user.tf`) : un utilisateur bootstrap
    (`batleforc`), champs `username`, `name`, `email`, `is_active`,
    `groups` (référence `authentik_group`). Entièrement absent du plan
    précédent.
- **Hors scope opérateur (mais motive le besoin multi-backend de secrets)**
  - `vault.tf` : fédère Authentik comme IdP OIDC pour Vault lui-même
    (`vault_jwt_auth_backend*`, mapping groupe Authentik -> policy Vault).
    C'est de la config côté Vault, pas côté Authentik — l'opérateur n'a
    pas à le gérer, mais c'est la raison pour laquelle le `SecretStore`
    doit pouvoir écrire vers Vault (workloads Vault-aware qui consomment
    le client secret).
  - Certificate key pair (`signing_key`) : toujours un lookup d'un cert
    existant (`authentik Self-signed Certificate`), jamais créé par
    Terraform. Même traitement que Flow : référencé par nom, pas de CRD
    dédiée en v1.

Conséquence directe : ~80% du module est le triplet
Provider+Application+PolicyBinding répété (OAuth2 ou Proxy).
L'allow-list namespace se mappe naturellement sur "quel namespace
possède quelle application".

## CRDs proposées (v1)

Scope tranché : `AuthentikInstance` et `AuthentikGroup` sont **globaux**
(cluster-scoped, concept d'identité partagé à l'échelle de
l'organisation) ; `AuthentikApplication` et `AuthentikAccessPolicy` sont
**namespaced** (possédés par l'équipe/namespace qui les crée, c'est ce
que l'allow-list gouverne).

- `AuthentikInstance` (cluster-scoped) : `url`,
  `tokenSecretRef {name,namespace,key}`, TLS.
- `AuthentikGroup` (cluster-scoped) : `name`, `isSuperuser`, `parentRef`
  (auto-référence pour la hiérarchie), `attributes`.
- `AuthentikApplication` (namespaced) : `instanceRef`, `name`, `slug`,
  `metaIcon`, `provider` (oneOf — `oauth2` **et** `proxy` implémentés en
  v1 pour couvrir la parité réelle avec Terraform, `saml`/`ldap`
  présents dans le schema mais non implémentés — erreur explicite si
  utilisés). Variante `proxy` : `internalHost`, `externalHost`,
  `authorizationFlow`, `invalidationFlow`, `outpostRef` (**optionnel** —
  voir `AuthentikOutpost` ci-dessous pour la résolution).
- `AuthentikOutpost` (cluster-scoped) : `name`, `type`
  (`proxy`/`ldap`/`radius`, seul `proxy` a un usage réel aujourd'hui),
  `config` (json libre, passthrough vers l'API Authentik). Volontairement
  minimal — rien dans le module Terraform ne crée d'outpost custom, cette
  CRD n'existe que pour couvrir le cas "référence explicite" ci-dessous.
  **Résolution de `outpostRef` sur un provider proxy** :
  - si `outpostRef` est renseigné → doit résoudre vers une
    `AuthentikOutpost` CR existante ; sinon erreur explicite (condition
    `Errored` + Event), pas de fallback silencieux.
  - si `outpostRef` est absent → attachement par défaut à l'outpost
    embarqué d'Authentik (résolu par nom, `authentik Embedded Outpost`),
    sans passer par une CR — c'est le comportement actuel de
    `longhorn.tf` et il reste le défaut pour ne rien casser.
- `AuthentikUser` (cluster-scoped, comme `AuthentikGroup`) : `username`,
  `name`, `email`, `isActive`, `groupRefs` (noms d'`AuthentikGroup`).
  **Pas de mot de passe/credential dans le spec** — Authentik gère ses
  propres flows d'invitation/reset ; si un bootstrap de mot de passe est
  un jour nécessaire, il passe par `SecretStore` (généré, jamais stocké
  en clair dans la CR), jamais par un champ `spec`.
- `AuthentikAccessPolicy` (namespaced) : équivalent direct de
  `authentik_policy_binding`. Champs : `applicationRef` (nom, **même
  namespace obligatoire**), `groupRef` (nom d'un `AuthentikGroup`
  cluster-scoped), `order`, `negate`. CRD séparée plutôt qu'un champ
  `accessBindings` inline sur `AuthentikApplication`, pour matcher 1:1
  la ressource Terraform et permettre au webhook d'admission de
  raisonner dessus indépendamment.

  **Confinement au namespace, deux couches** :
  1. *Structurel* : `applicationRef` est une string (nom seul), aucun
     champ namespace dans le schema — il n'existe simplement aucun moyen
     d'exprimer une référence cross-namespace dans la CR.
  2. *Résolution* : le reconciler lit toujours l'application via
     `Api::<AuthentikApplication>::namespaced(policy.metadata.namespace)`
     — jamais un client cluster-wide — donc même un nom qui existe dans
     un autre namespace ne peut pas être trouvé par accident. Si le nom
     ne résout à rien dans le namespace propre de la policy, la CR passe
     en condition `Errored` (`reason: ApplicationRefNotFound`) avec un
     `Event` explicite ; pas de retry silencieux indéfini, pas de
     fallback vers un autre namespace.
  Optionnel en défense en profondeur : une règle CEL
  (`x-kubernetes-validations`) sur le CRD peut rejeter à l'admission tout
  `applicationRef` qui contiendrait un `/` ou un format
  `namespace/name`, pour fermer la porte même à une future évolution du
  schema qui ajouterait par erreur un tel champ.
- `AuthentikNamespacePolicy` (cluster-scoped) : règles allow/deny par
  namespace sur qui peut créer `AuthentikApplication`/
  `AuthentikAccessPolicy`. Default-deny **dès qu'au moins une
  `AuthentikNamespacePolicy` existe dans le cluster** — y compris pour un
  namespace non couvert par aucune règle. Tant qu'aucune
  `AuthentikNamespacePolicy` n'existe encore (cluster non bootstrappé),
  tous les namespaces sont autorisés : pas de policy à faire respecter.
  Appliquée via
  `ValidatingWebhookConfiguration` (endpoint admission maison, servi par
  l'opérateur). Renommée depuis "AuthentikAccessPolicy" pour éviter la
  collision avec la CRD ci-dessus qui, elle, correspond au
  `policy_binding` Terraform.
- Flow : **référencé** par slug (string, résolu au reconcile), pas de
  CRUD CRD dédiée en v1 — reflète l'usage Terraform actuel (un seul flow
  custom, le reste en lookup). CRUD complet = v2 si besoin émerge.
- `AuthentikBrand` (cluster-scoped, cf. rectification ci-dessous) :
  correction par rapport à une version précédente de ce doc qui reléguait
  Brand en v2 — Terraform gère bien `authentik_brand` avec un champ
  `default`, ça rentre dans le scope Route C. Champs : `domain`,
  `default` (bool), `brandingTitle`, `brandingLogo`, `brandingFavicon`,
  `brandingDefaultFlowBackground`, `defaultApplicationRef`,
  `flowAuthentication`/`flowInvalidation`/`flowRecovery`/
  `flowUnenrollment`/`flowUserSettings` (slugs, pas de CRD Flow derrière
  en v1). Cluster-scoped comme `AuthentikInstance`/`AuthentikGroup` :
  un brand est rattaché à un domaine à l'échelle du cluster/instance, pas
  à un namespace applicatif.

  **Élection du brand par défaut** (un seul `default=true` valide à la
  fois, côté Authentik comme côté CRD) :
  1. `spec.default: true` déclenché sur une CR → le controller liste
     toutes les `AuthentikBrand` avec `spec.default: true`, trie par
     `creationTimestamp` (tie-break sur le nom si égalité). Seule la
     plus ancienne ("gagnante") va effectivement toucher l'API
     Authentik ; c'est une résolution côté Kubernetes (source de vérité
     stable), pas une course contre l'état Authentik distant.
  2. La CR gagnante réconcilie : si le brand actuellement `default` côté
     Authentik n'est géré par aucune `AuthentikBrand` CR (i.e. c'est le
     brand par défaut natif/bootstrap d'Authentik), l'opérateur le
     désactive (`default=false`) puis active le sien.
  3. Toute autre CR `spec.default: true` (perdante du tri) passe en
     condition `Errored` (`reason: DefaultBrandConflict`) **sans**
     appeler l'API Authentik, et un `Event` Kubernetes explicite
     (`type: Warning`) est émis sur cette CR, nommant la CR gagnante en
     conflit. Pas de retry en boucle tant que le conflit n'est pas
     résolu côté utilisateur (passer l'une des deux CR à `default:
     false`, ou la supprimer) — le controller doit watcher les deux CR
     pour re-réconcilier dès que l'une change.
  4. **Suppression de la CR gagnante (finalizer)** : l'opérateur
     supprime le brand correspondant côté Authentik (comportement
     standard delete-via-finalizer). Le controller re-liste alors les
     `AuthentikBrand` restantes avec `spec.default: true` :
     - s'il y en a une (perdante précédente, ou nouvelle CR créée
       entre-temps), l'élection normale s'applique et elle devient la
       nouvelle gagnante ;
     - s'il n'y en a aucune, **le brand par défaut natif Authentik
       n'est volontairement pas réactivé** — pas de restauration
       automatique. L'instance reste sans brand `default` explicite
       tant qu'aucune CR ne le redemande.

## Convention d'erreurs : codes stables

Règle : **aucune chaîne de raison/erreur écrite en dur au point d'appel**.
Toute raison exposée (condition de status — bloquante ou juste
avertissement —, `Event` Kubernetes, refus de webhook d'admission, champ
d'erreur otel) référence un code unique défini une seule fois dans le
code.

- Un seul enum `domain::error::ReasonCode` (`Debug, Clone, Copy,
  PartialEq, Eq`) — renommé depuis `ErrorCode` : il couvre maintenant
  aussi bien les raisons bloquantes (`Ready: False`) que les avertissements
  non-bloquants (cf. `NoAccessPolicyBound` ci-dessous, où l'objet reste
  `Ready: True` mais affiche quand même un signal stable). Une variante
  par famille, `as_str(&self) -> &'static str` qui rend le nom PascalCase
  (déjà compatible avec la contrainte de format `reason` de Kubernetes
  pour les `Condition`/`Event`), et `severity(&self) -> Severity`
  (`Blocking | Advisory`) pour que les call sites sachent s'ils doivent
  aussi flipper `Ready` à `False`. Un seul catalogue, une seule source de
  vérité — pas de second enum parallèle pour les warnings.
- **Contrainte au niveau des types, pas juste une convention** : les
  helpers qui posent une condition de status ou émettent un `Event`
  (`set_condition(..., code: ReasonCode, ...)`, `emit_event(..., code:
  ReasonCode, ...)`) n'acceptent que `ReasonCode` en paramètre, jamais
  `&str`. Un reconciler ne peut donc pas compiler s'il invente une
  chaîne de raison à la volée.
- Le webhook d'admission (`AuthentikNamespacePolicy`) et les reconcilers
  doivent réutiliser le **même** code pour la même règle métier — ex. un
  refus webhook et une condition de status qui parlent tous les deux
  d'un namespace non autorisé utilisent `NamespaceNotAllowed`, pas deux
  variantes différentes.
- Catalogue amorcé jusqu'ici (à étendre au fil de l'implémentation, mais
  toujours en ajoutant à cet enum, jamais en ligne) :
  - `DefaultBrandConflict` (Blocking) — deux `AuthentikBrand` avec
    `default: true`.
  - `ApplicationRefNotFound` (Blocking) —
    `AuthentikAccessPolicy.applicationRef` ne résout à rien dans le
    namespace propre de la policy.
  - `NamespaceNotAllowed` (Blocking) — refus par
    `AuthentikNamespacePolicy` (webhook et/ou reconciler).
  - `OutpostRefNotFound` (Blocking) — `outpostRef` d'un provider proxy ne
    résout à aucune `AuthentikOutpost`.
  - `AuthentikObjectAlreadyExists` (Blocking) — collision de nom/slug
    avec un objet Authentik existant non tracké par cette CR (cf.
    "Modèle de status commun" ci-dessous).
  - `NoAccessPolicyBound` (Advisory) — `AuthentikApplication` active
    sans aucune `AuthentikAccessPolicy` pointant vers elle.
  - `Reconciled` (Advisory) — ajouté pendant le scaffolding : code de
    succès générique pour `Ready: True`, puisque `Condition::ready`
    exige toujours un `ReasonCode` (contrainte de type), même sur le
    chemin nominal. Un seul code partagé plutôt que chaque reconciler
    n'invente le sien.

## Modèle de status commun

Toutes les CRD partagent la même forme de `status` (définie une fois dans
`domain`, réutilisée par chaque reconciler) :

```
status:
  observedGeneration: int64
  authentikId: string | null   # PK côté Authentik ; présent = cette CR
                                 # possède réellement l'objet distant
  conditions:
    - type: Ready
      status: "True" | "False" | "Unknown"
      reason: ReasonCode        # cf. section précédente
      message: string
      lastTransitionTime: timestamp
      observedGeneration: int64
    # + conditions additionnelles pour les avertissements non-bloquants
    # (même schema, mais Ready reste True à côté)
```

`authentikId` est le champ pivot qui répond aux deux points ci-dessous.

### Collision avec un objet existant → rejet, jamais d'adoption implicite

Séquence à la première réconciliation d'une CR (`status.authentikId` vide) :

1. Le reconciler tente directement la **création** côté Authentik (pas de
   `list`-puis-`create` : un lookup préalable par nom/slug ouvre une
   fenêtre de course entre plusieurs reconciles concurrents). Pas de
   pré-vérification "est-ce que ça existe déjà" — c'est l'API Authentik
   qui tranche.
2. Si la création réussit → le PK retourné est écrit dans
   `status.authentikId`. C'est la seule façon dont une CR "possède"
   désormais un objet Authentik.
3. Si l'API Authentik refuse pour cause de doublon (slug/nom déjà pris
   par un objet que cette CR ne possède pas) → `Ready: False`,
   `reason: AuthentikObjectAlreadyExists`, `authentikId` reste vide, un
   `Event` explicite est émis. Pas de retry en boucle serrée (cf.
   `Politique de mutation` pour le backoff) — le conflit doit être résolu
   par un humain (renommer, ou confirmer qu'il s'agit bien du même objet
   via l'import).
4. **Seul chemin légitime pour qu'une CR se rattache à un objet Authentik
   préexistant** : l'outil d'import de la Route C écrit `status.authentikId`
   directement au moment où il génère la CR à partir de l'état Authentik
   réel — jamais le reconciler ne le déduit lui-même en matchant un nom.

### Avertissement non-bloquant : application sans access policy

Conformément à votre choix (documenter et laisser faire, pas bloquer) :
tant qu'une `AuthentikApplication` n'a aucune `AuthentikAccessPolicy` qui
la référence, le reconciler ajoute une condition
`type: NoAccessPolicyBound, status: "True", reason: NoAccessPolicyBound`
à côté de `Ready: True` (l'application fonctionne, elle est juste ouverte
par défaut côté Authentik — comportement natif, pas un bug de
l'opérateur). Documenté en gras dans le guide "premier parcours
applicatif" plutôt que caché dans une note de bas de page.

## Politique de mutation : patch-first, delete-recreate seulement si
   nécessaire

Décision : **la CR est autoritative** — pas de détection de drift qui
laisserait un changement manuel côté Authentik survivre à un reconcile.
Chaque reconcile calcule le diff entre le `spec` désiré et l'état actuel
lu via `AuthentikGateway::get(authentikId)`, puis envoie un **PATCH**
(update partiel), jamais un `PUT`/replace complet et jamais un
delete-recreate par défaut.

- Delete-recreate n'est déclenché que pour le seul cas où un patch ne
  peut structurellement pas s'appliquer : changer la **variante** du
  `provider` oneOf (`oauth2` → `proxy` par ex.) — on ne peut pas
  transformer un provider OAuth2 en provider Proxy en place côté
  Authentik. Comme cette opération régénère forcément un nouveau
  `client_secret`/casse potentiellement des sessions actives, elle exige
  une confirmation explicite : sans une annotation du type
  `authentik.weebo.io/allow-disruptive-update: "true"` présente sur la CR
  au moment du reconcile, l'opérateur refuse et pose `Ready: False`
  plutôt que de recréer silencieusement.
- Tout le reste (redirect URIs, meta_icon, flows référencés, policy
  binding order, etc.) reste du PATCH in-place.

## Sécurité opérationnelle & déploiement

- **`ValidatingWebhookConfiguration.failurePolicy: Fail`.** Si le pod
  webhook est injoignable, toute création/mise à jour d'`AuthentikApplication`/
  `AuthentikAccessPolicy` est bloquée cluster-wide jusqu'à son retour —
  l'allow-list est le mécanisme de sécurité central du projet, on ne la
  laisse pas se contourner silencieusement par un pod down. Conséquence
  directe : le webhook doit tourner en plusieurs replicas avec
  `PodDisruptionBudget`, sous peine de transformer un simple redéploiement
  de l'opérateur en incident.
- **Portée du webhook sur les verbes** : validation sur `CREATE` et
  `UPDATE`, jamais sur `DELETE`. Révoquer l'accès d'un namespace (retirer/
  modifier son `AuthentikNamespacePolicy`) ne doit jamais empêcher le
  nettoyage (finalizer) des ressources déjà créées — sinon la révocation
  d'accès stranderait des objets impossibles à supprimer proprement.
- **HA du controller manager : leader election standard.** Un seul
  reconciler actif à la fois même avec plusieurs replicas — nécessaire
  pour que le pattern "tentative de création directe, pas de
  lookup-puis-create" (cf. "Modèle de status commun") reste sûr en
  multi-replica ; sans ça, deux reconcilers actifs pourraient toujours
  tenter une création concurrente sur le même objet.
- **RBAC livré dans le chart Helm, pas laissé à l'improvisation** : un
  `Role`/`RoleBinding` namespaced template donnant seulement les verbes
  sur `AuthentikApplication`/`AuthentikAccessPolicy` ; `AuthentikInstance`/
  `AuthentikGroup`/`AuthentikUser`/`AuthentikNamespacePolicy` ne sont
  accessibles que via RBAC cluster-scoped (déjà vrai structurellement
  puisqu'ils sont cluster-scoped, mais le chart documente/exemplifie la
  frontière plutôt que de la laisser implicite).
- **Intégration ArgoCD** : le cluster cible étant géré en GitOps/ArgoCD,
  le chart Helm embarque un `health.lua` custom (ConfigMap dans le
  `resource.customizations` d'ArgoCD) qui lit `status.conditions[type=Ready]`
  pour chaque CRD — sans ça, ArgoCD ne sait pas interpréter un CRD
  inconnu et affiche chaque ressource comme "Progressing" indéfiniment
  même une fois correctement synchronisée.

**Laissé ouvert pour plus tard** (noté, pas bloquant pour le scope v1) :
scoper `AuthentikNamespacePolicy` par `AuthentikInstance` en plus du
namespace (utile seulement à partir du moment où plusieurs instances
Authentik cohabitent) ; convention de nommage par défaut des Secrets K8s/
chemins Vault produits par `SecretStore` (à trancher au moment d'écrire
l'adapter, pas un choix structurant).

## Architecture hexagonale

- `domain/` : entités, logique d'évaluation de l'allow-list, polymorphisme
  provider. Zéro dépendance `kube`/`http` — testable sans cluster.
- `application/` : use-cases (`ReconcileApplication`, `ReconcileGroup`,
  `EvaluateAdmission`) qui orchestrent domain + ports.
- `ports` : trait `AuthentikGateway`, trait `SecretStore`.
- `adapters/inbound` : `Controller` kube.rs par CRD (finalizers + status
  conditions), webhook d'admission (`axum`), bootstrap otel.
- `adapters/outbound` : `AuthentikHttpGateway` (wrap `authentik-client`),
  `K8sSecretStore`, `VaultSecretStore`.
- Génération CRD : crate binaire `crdgen` (Rust, via
  `CustomResourceExt::crd()`), invoquée directement par `task recu`
  (placeholder déjà présent dans `Taskfile.yaml`) — pas de couche
  `xtask` supplémentaire, `task` (go-task) est déjà l'unique point
  d'entrée d'automatisation de ce repo.
- Stockage secret : uniquement K8s Secret + Vault en natif. Pour "autre",
  s'appuyer sur External Secrets Operator plutôt que construire un
  troisième adapter natif (évite l'abstraction spéculative).

## Documentation (prérequis, pas une option v2)

Un site de doc **Fumadocs** (Next.js) vit dans le repo — cohérent avec
`node = "24"` déjà présent dans `mise.toml`. Deux natures de contenu, pas
confondues :

- **Référence générée (dynamique, jamais éditée à la main)** : une page
  par CRD, table des champs (nom, type, requis/optionnel, défaut,
  description) + un formulaire interactif qui aide à remplir la CR et
  produit le YAML correspondant en direct.
- **Guides rédigés (statiques, mais obligatoires)** : install, connexion
  à une vraie instance Authentik, premier parcours applicatif.

### Pipeline de génération

`task recu` (déjà le point d'entrée "régénère les specs" dans
`Taskfile.yaml`) s'étend en deux étapes chaînées :

1. `task recu` invoque le binaire `crdgen` (Rust) → `deploy/crd/*.yaml`.
2. Un step de génération doc (Rust ou script Node dans `docs/scripts/`)
   lit ces CRD YAML (schema OpenAPI v3 structurel) et émet, par CRD :
   - `docs/content/crds/<Kind>.mdx` — texte généré à partir des
     `description` des champs (donc les `#[schemars(description = "...")]`
     / doc-comments Rust deviennent la doc utilisateur ; une CR sans
     description de champ correcte produit une doc pauvre, ce qui est
     un signal utile en review).
   - `docs/public/crd-schemas/<kind>.schema.json` — le schema
     consommé côté client par le composant de formulaire.

**Risque à anticiper, pas à ignorer** : le schema structurel d'un CRD
Kubernetes n'est pas du JSON Schema standard (extensions
`x-kubernetes-preserve-unknown-fields`, `x-kubernetes-int-or-string`, et
le `oneOf` du champ `provider` est un pattern maison, pas un `oneOf`
JSON Schema natif tant que la structural schema de k8s ne le supporte
pas directement). Une lib de formulaire générique (type
`react-jsonschema-form`) ne va probablement pas gober ça telle quelle —
prévoir soit un step de "nettoyage" du schema avant de le donner à la
lib, soit un petit renderer de formulaire maison qui ne couvre que le
sous-ensemble de JSON Schema réellement utilisé par nos CRD (plus
réaliste vu qu'on maîtrise les schémas des deux côtés).

### Definition of done

**Une CRD n'est pas considérée livrée tant que sa page de référence +
son formulaire ne sont pas générés.** Appliqué en CI : un check
"regen-and-diff" (régénère `docs/content/crds/` et
`docs/public/crd-schemas/` à partir de `deploy/crd/`, échoue si ça
diffère de ce qui est commité) — même mécanique que pour n'importe quel
artefact généré qu'on garde versionné.

### Guides obligatoires (contenu attendu, rédigé)

- Installer l'opérateur + les CRD via **Helm chart** (décision prise
  ci-dessous) : `helm install`, structure du chart, valeurs exposées.
- Connecter une `AuthentikInstance` à un vrai serveur Authentik : créer
  le secret contenant le token API, manifeste minimal `url` +
  `tokenSecretRef`, options TLS.
- Premier parcours applicatif de bout en bout (créer `AuthentikGroup`,
  `AuthentikApplication` oauth2, `AuthentikAccessPolicy`), repris sur
  l'exemple réel harbor pour rester ancré dans le vécu Terraform.
- Fonctionnement de l'allow-list (`AuthentikNamespacePolicy`) : comment
  un namespace obtient le droit de créer des ressources, comportement
  par défaut (deny dès qu'une policy existe quelque part dans le
  cluster ; allow tant qu'aucune n'existe encore), message d'erreur reçu
  si refusé.

Ouvert / pas tranché ici : où le site Fumadocs est hébergé/déployé
(GitHub Pages, Vercel, self-hosted) — pas bloquant pour la conception du
pipeline de génération, à trancher au moment du scaffolding du site.

## Stratégie de tests : une factory de test réutilisable

Objectif exprimé : pouvoir vérifier "que tout marche vraiment", y compris
sans dépendre d'un vrai cluster complet ni d'une vraie instance Authentik
à chaque run. Quatre couches, du plus rapide/isolé au plus proche du
réel :

1. **Unit domain** : logique pure (élection du brand par défaut,
   évaluation de l'allow-list, calcul de diff patch) testée sans I/O du
   tout — c'est tout l'intérêt d'avoir gardé `domain/` sans dépendance
   `kube`/`http`.
2. **Contrat adapter sortant** : `AuthentikHttpGateway` testé contre
   `wiremock` (réponses HTTP scriptées, y compris les cas 409/conflit du
   "Modèle de status commun" et les erreurs Authentik réalistes).
3. **Intégration controller, API Kubernetes légère** : exactement l'idée
   proposée — pas un vrai cluster avec kubelet/scheduler/pods, juste
   `kube-apiserver` + `etcd` réels (le pattern `envtest` de l'écosystème
   kubebuilder, utilisable depuis Rust via les binaires téléchargés en
   `KUBEBUILDER_ASSETS` — aucune dépendance Docker/Podman, donc ça tourne
   même dans un environnement Che sans accès root/conteneurs). Les CRD
   réelles sont installées, la validation/le defaulting/les
   status-subresources sont ceux d'un vrai apiserver Kubernetes, mais
   rien n'exécute de pod applicatif. Le reconciler tourne pour de vrai
   contre cet apiserver, avec `AuthentikGateway` remplacé par le mock
   `wiremock` de la couche 2 (pas de vraie instance Authentik requise).
4. **E2E complet, périodique plutôt qu'à chaque PR** : vrai cluster
   (`kind` en CI) + vraie instance Authentik jetable (conteneur), pour
   valider le chemin de bout en bout de temps en temps sans ralentir
   chaque PR.

**Factory réutilisable** : un crate `testkit` interne qui expose les
helpers communs (démarrer l'apiserver éphémère de la couche 3, appliquer
les CRD, enregistrer un `AuthentikGateway` scripté, assertions sur
`status.conditions`/`authentikId`, teardown propre) — chaque nouveau
reconciler écrit ses tests d'intégration contre cette factory plutôt que
de réinventer le bootstrap à chaque fois.

## CI / Release

Pipelines écrits en syntaxe **GitHub Actions**, compatible **Forgejo
Actions** (Forgejo implémente le même format de workflow) — l'hébergement
définitif (github.com vs Forgejo self-hosted) n'est pas tranché, donc on
évite les actions du marketplace GitHub sans équivalent Forgejo pour
rester portable entre les deux.

Étapes attendues :
- `cargo fmt --check`, `cargo clippy`, `cargo test` (couches 1+2 de la
  stratégie de tests).
- Tests d'intégration couche 3 (`KUBEBUILDER_ASSETS` téléchargés en CI).
- Check "regen-and-diff" : `task recu` (crdgen + doc-gen) ne doit rien
  changer par rapport à ce qui est commité.
- Lint/package du chart Helm.
- Build de l'image de l'opérateur, bump de version piloté par `cog.toml`
  (déjà en place) pour le versioning/changelog, publication de l'image et
  du chart.
- E2E périodique (couche 4) sur un déclenchement séparé (cron ou manuel),
  pas sur chaque PR.

## Décisions

1. **TLS du webhook d'admission : cert-manager requis.** L'opérateur
   déclare une ressource `Certificate` (cert-manager) et consomme le
   secret TLS résultant ; pas de gestion de cert auto-signé maison.
2. **Scope provider v1 (révisé) : `oauth2` + `proxy` implémentés,
   `saml`/`ldap` stubbés.** Correction d'une décision précédente qui
   stubait `proxy` par erreur — `longhorn.tf` utilise réellement
   `authentik_provider_proxy` aujourd'hui, donc la parité Route C
   l'exige. `saml`/`ldap` restent des stubs de schema (non implémentés)
   car rien dans le module ne les utilise.
3. **Route retenue : Route C — migration à parité.** Cf. section
   "Trois routes possibles" ci-dessous pour le détail.

4. **Scope des CRD confirmé** : `AuthentikInstance`/`AuthentikGroup`
   cluster-scoped ; `AuthentikApplication`/`AuthentikAccessPolicy`
   namespaced. Voir section CRDs ci-dessus pour le détail (renommage de
   l'allow-list en `AuthentikNamespacePolicy` pour éviter la collision
   avec `AuthentikAccessPolicy` = équivalent `policy_binding`).
5. **`AuthentikUser` est dans le scope de l'opérateur.** CRD
   cluster-scoped au même titre que `AuthentikGroup`, avec le même
   pipeline reconcile/finalizer. Aucun champ credential dans le spec
   (cf. section CRDs).
6. **Packaging : Helm chart.** L'opérateur, ses CRD et son
   `ValidatingWebhookConfiguration` s'installent via un chart Helm
   (plutôt que manifestes bruts ou Kustomize) — c'est aussi ce que le
   guide d'installation documente.
7. **Documentation : Fumadocs, référence générée + guides rédigés,
   traité comme prérequis de livraison.** Voir section "Documentation"
   ci-dessus.
8. **Application sans access policy : avertissement, pas blocage.**
   Condition `NoAccessPolicyBound` (Advisory), `Ready` reste `True`.
   Documenté explicitement dans le guide.
9. **Collision de nom/slug avec un objet Authentik non tracké : rejet.**
   Jamais d'adoption implicite par matching de nom — seul l'outil
   d'import peut rattacher une CR à un `authentikId` existant.
10. **Status commun avec `authentikId` : adopté.** Cf. "Modèle de status
    commun".
11. **Mutation : CR autoritative, patch-first.** Delete-recreate limité
    au changement de variante du `provider`, et seulement derrière
    l'annotation explicite `authentik.weebo.io/allow-disruptive-update`.
12. **Tests : factory `testkit` + apiserver éphémère type envtest.** Cf.
    "Stratégie de tests".
13. **CI : GitHub Actions, syntaxe compatible Forgejo Actions.** Cf.
    "CI / Release".
14. **`provider` n'est pas un enum Rust taggé, malgré ce que ce document
    disait plus haut.** Confirmé pendant le scaffolding : `kube-core` ne
    sait pas fusionner le schema d'un enum interne-taggé dont les
    variantes portent des valeurs de discriminant différentes (chaque
    variante donne une schema `enum: [...]` différente pour la propriété
    `kind`, et `CustomResourceExt::crd()` panique : "Property 'kind' ...
    must be identical"). `AuthentikApplication.spec.provider` est donc une
    struct plate `{ kind: oauth2|proxy|saml|ldap, oauth2: Option<..>,
    proxy: Option<..> }` — le "exactly one of, cohérent avec kind" est
    validé par le reconciler, pas par le schema CRD. C'est le contournement
    standard dans l'écosystème kube.rs pour ce cas précis.
14. **Webhook `failurePolicy: Fail`.** Sécurité de l'allow-list priorisée
    sur la disponibilité ; implique webhook multi-replica + PDB.
15. **Cluster cible géré en ArgoCD/GitOps.** Le chart Helm doit fournir
    un `health.lua` custom pour chaque CRD. Cf. "Sécurité opérationnelle
    & déploiement".

## Trois routes possibles

**Route A — Vertical slice MVP.** Un seul couple de CRD (Application+
Provider OAuth2), K8s Secret uniquement, pas de webhook/allow-list au
départ, contre une vraie instance Authentik de dev. Feedback rapide, mais
la priorité affichée (sécurité/allow-list) arrive en dernier.

**Route B — Domain-first.** Domaine pur + ports + harnais de tests de
contrat (wiremock) contre une API Authentik mockée, avant tout cluster
réel. Force à bien concevoir le polymorphisme provider et la logique
d'allow-list dès le départ. Démo sur cluster réel plus tardive ; risque
de sur-concevoir des ports pour des cas qui ne se matérialisent pas.

**Route C — Migration à parité (recommandée).** Scope des CRD limité
strictement à ce que l'inventaire Terraform ci-dessus prouve nécessaire.
Un outil d'import one-shot (via `authentik-client`) lit l'état Authentik
existant et génère le YAML des CRD, puis migration namespace par
namespace, l'allow-list garantissant que Terraform et l'opérateur ne
touchent jamais le même namespace en même temps. Scope borné, allow-list
structurante dès le jour 1, progression mesurable directement contre
l'objectif de remplacement de Terraform.

Recommandation : Route C, en empruntant à B la rigueur sur le schema
`oneOf` du provider (peu coûteux à bien faire tôt, coûteux à corriger
après coup).
