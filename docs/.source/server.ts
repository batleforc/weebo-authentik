// @ts-nocheck
import * as __fd_glob_22 from "../content/docs/guides/release.mdx?collection=docs"
import * as __fd_glob_21 from "../content/docs/guides/migrate-from-terraform.mdx?collection=docs"
import * as __fd_glob_20 from "../content/docs/guides/install.mdx?collection=docs"
import * as __fd_glob_19 from "../content/docs/guides/first-application.mdx?collection=docs"
import * as __fd_glob_18 from "../content/docs/guides/cutting-a-release.mdx?collection=docs"
import * as __fd_glob_17 from "../content/docs/guides/connect-instance.mdx?collection=docs"
import * as __fd_glob_16 from "../content/docs/guides/build-a-manifest.mdx?collection=docs"
import * as __fd_glob_15 from "../content/docs/guides/allow-list.mdx?collection=docs"
import * as __fd_glob_14 from "../content/docs/crds/index.mdx?collection=docs"
import * as __fd_glob_13 from "../content/docs/crds/authentikuser.mdx?collection=docs"
import * as __fd_glob_12 from "../content/docs/crds/authentikscopemapping.mdx?collection=docs"
import * as __fd_glob_11 from "../content/docs/crds/authentikoutpost.mdx?collection=docs"
import * as __fd_glob_10 from "../content/docs/crds/authentiknamespacepolicy.mdx?collection=docs"
import * as __fd_glob_9 from "../content/docs/crds/authentikinstance.mdx?collection=docs"
import * as __fd_glob_8 from "../content/docs/crds/authentikgroup.mdx?collection=docs"
import * as __fd_glob_7 from "../content/docs/crds/authentikflow.mdx?collection=docs"
import * as __fd_glob_6 from "../content/docs/crds/authentikbrand.mdx?collection=docs"
import * as __fd_glob_5 from "../content/docs/crds/authentikapplication.mdx?collection=docs"
import * as __fd_glob_4 from "../content/docs/crds/authentikaccesspolicy.mdx?collection=docs"
import * as __fd_glob_3 from "../content/docs/index.mdx?collection=docs"
import { default as __fd_glob_2 } from "../content/docs/guides/meta.json?collection=docs"
import { default as __fd_glob_1 } from "../content/docs/crds/meta.json?collection=docs"
import { default as __fd_glob_0 } from "../content/docs/meta.json?collection=docs"
import { server } from 'fumadocs-mdx/runtime/server';
import type * as Config from '../source.config';

const create = server<typeof Config, import("fumadocs-mdx/runtime/types").InternalTypeConfig & {
  DocData: {
  }
}>();

export const docs = await create.docs("docs", "content/docs", {"meta.json": __fd_glob_0, "crds/meta.json": __fd_glob_1, "guides/meta.json": __fd_glob_2, }, {"index.mdx": __fd_glob_3, "crds/authentikaccesspolicy.mdx": __fd_glob_4, "crds/authentikapplication.mdx": __fd_glob_5, "crds/authentikbrand.mdx": __fd_glob_6, "crds/authentikflow.mdx": __fd_glob_7, "crds/authentikgroup.mdx": __fd_glob_8, "crds/authentikinstance.mdx": __fd_glob_9, "crds/authentiknamespacepolicy.mdx": __fd_glob_10, "crds/authentikoutpost.mdx": __fd_glob_11, "crds/authentikscopemapping.mdx": __fd_glob_12, "crds/authentikuser.mdx": __fd_glob_13, "crds/index.mdx": __fd_glob_14, "guides/allow-list.mdx": __fd_glob_15, "guides/build-a-manifest.mdx": __fd_glob_16, "guides/connect-instance.mdx": __fd_glob_17, "guides/cutting-a-release.mdx": __fd_glob_18, "guides/first-application.mdx": __fd_glob_19, "guides/install.mdx": __fd_glob_20, "guides/migrate-from-terraform.mdx": __fd_glob_21, "guides/release.mdx": __fd_glob_22, });