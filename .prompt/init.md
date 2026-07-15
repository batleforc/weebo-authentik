# Init prompt pour le projet Authentik Kubernetes operator

L'objectif de ce projet est de creer un opérateur kubernetes pour configurer Authentik. Mais aussi et surtout de rendre ce projet maintenable au maximum et automatiser. Les CRD doivent être générable via script au maximum et permettre une configurabilité au maximum en gardant une sécurité importante.

Par exemple, la base d'une configuration serais l'instance Authentik comportant les information pour ce connecter (URL, secret pour avoir l'identifiant/pwd, etc) et aussi un concept d'allow list. Par exemple, je veux pouvoir creer une nouvelle application avec un provider oauth dans le namespace A mais pas dans le B. Cette allow list pourrait étre forcer cia un mécanisme de webhoon de validation indiquant a celui qui tente de le creer que ce namespace n'est pas autorisé a creer cette objet. 

L'objectif est de remplacer toute ma configuration terraform https://github.com/batleforc/weebo-si/tree/develop/2.terra/auth/terra-map par cette opérateur.

## Stack technique

- Language : Rust
- Architecture: Hexagonale
- Lib Kubernetes : [KubeRS](https://kube.rs/)
- Client authentik : <https://github.com/goauthentik/client-rust>
- Secret Storage : 
    - Kubernetes Secret
    - Vault
    - Autre ? 
- observabilité: Trace et Metric exposé via Otel.


Dans un premier temps, ta missions est d'affiner ce plan et le compléter puis te proposer 3 routes pour compléter cete objectif.