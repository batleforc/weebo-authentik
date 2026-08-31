// @ts-nocheck
import { browser } from 'fumadocs-mdx/runtime/browser';
import type * as Config from '../source.config';

const create = browser<typeof Config, import("fumadocs-mdx/runtime/types").InternalTypeConfig & {
  DocData: {
  }
}>();
const browserCollections = {
  docs: create.doc("docs", {"index.mdx": () => import("../content/docs/index.mdx?collection=docs"), "crds/authentikaccesspolicy.mdx": () => import("../content/docs/crds/authentikaccesspolicy.mdx?collection=docs"), "crds/authentikapplication.mdx": () => import("../content/docs/crds/authentikapplication.mdx?collection=docs"), "crds/authentikbrand.mdx": () => import("../content/docs/crds/authentikbrand.mdx?collection=docs"), "crds/authentikgroup.mdx": () => import("../content/docs/crds/authentikgroup.mdx?collection=docs"), "crds/authentikinstance.mdx": () => import("../content/docs/crds/authentikinstance.mdx?collection=docs"), "crds/authentiknamespacepolicy.mdx": () => import("../content/docs/crds/authentiknamespacepolicy.mdx?collection=docs"), "crds/authentikoutpost.mdx": () => import("../content/docs/crds/authentikoutpost.mdx?collection=docs"), "crds/authentikuser.mdx": () => import("../content/docs/crds/authentikuser.mdx?collection=docs"), "guides/allow-list.mdx": () => import("../content/docs/guides/allow-list.mdx?collection=docs"), "guides/connect-instance.mdx": () => import("../content/docs/guides/connect-instance.mdx?collection=docs"), "guides/cutting-a-release.mdx": () => import("../content/docs/guides/cutting-a-release.mdx?collection=docs"), "guides/first-application.mdx": () => import("../content/docs/guides/first-application.mdx?collection=docs"), "guides/install.mdx": () => import("../content/docs/guides/install.mdx?collection=docs"), "guides/migrate-from-terraform.mdx": () => import("../content/docs/guides/migrate-from-terraform.mdx?collection=docs"), "guides/release.mdx": () => import("../content/docs/guides/release.mdx?collection=docs"), }),
};
export default browserCollections;