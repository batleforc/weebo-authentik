// @ts-nocheck
import * as __fd_glob_15 from "../content/docs/guides/release.mdx?collection=docs"
import * as __fd_glob_14 from "../content/docs/guides/migrate-from-terraform.mdx?collection=docs"
import * as __fd_glob_13 from "../content/docs/guides/install.mdx?collection=docs"
import * as __fd_glob_12 from "../content/docs/guides/first-application.mdx?collection=docs"
import * as __fd_glob_11 from "../content/docs/guides/cutting-a-release.mdx?collection=docs"
import * as __fd_glob_10 from "../content/docs/guides/connect-instance.mdx?collection=docs"
import * as __fd_glob_9 from "../content/docs/guides/allow-list.mdx?collection=docs"
import * as __fd_glob_8 from "../content/docs/crds/authentikuser.mdx?collection=docs"
import * as __fd_glob_7 from "../content/docs/crds/authentikoutpost.mdx?collection=docs"
import * as __fd_glob_6 from "../content/docs/crds/authentiknamespacepolicy.mdx?collection=docs"
import * as __fd_glob_5 from "../content/docs/crds/authentikinstance.mdx?collection=docs"
import * as __fd_glob_4 from "../content/docs/crds/authentikgroup.mdx?collection=docs"
import * as __fd_glob_3 from "../content/docs/crds/authentikbrand.mdx?collection=docs"
import * as __fd_glob_2 from "../content/docs/crds/authentikapplication.mdx?collection=docs"
import * as __fd_glob_1 from "../content/docs/crds/authentikaccesspolicy.mdx?collection=docs"
import * as __fd_glob_0 from "../content/docs/index.mdx?collection=docs"
import { server } from 'fumadocs-mdx/runtime/server';
import type * as Config from '../source.config';

const create = server<typeof Config, import("fumadocs-mdx/runtime/types").InternalTypeConfig & {
  DocData: {
  }
}>();

export const docs = await create.docs("docs", "content/docs", {}, {"index.mdx": __fd_glob_0, "crds/authentikaccesspolicy.mdx": __fd_glob_1, "crds/authentikapplication.mdx": __fd_glob_2, "crds/authentikbrand.mdx": __fd_glob_3, "crds/authentikgroup.mdx": __fd_glob_4, "crds/authentikinstance.mdx": __fd_glob_5, "crds/authentiknamespacepolicy.mdx": __fd_glob_6, "crds/authentikoutpost.mdx": __fd_glob_7, "crds/authentikuser.mdx": __fd_glob_8, "guides/allow-list.mdx": __fd_glob_9, "guides/connect-instance.mdx": __fd_glob_10, "guides/cutting-a-release.mdx": __fd_glob_11, "guides/first-application.mdx": __fd_glob_12, "guides/install.mdx": __fd_glob_13, "guides/migrate-from-terraform.mdx": __fd_glob_14, "guides/release.mdx": __fd_glob_15, });