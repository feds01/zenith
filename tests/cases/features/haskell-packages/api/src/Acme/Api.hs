{-# LANGUAGE PackageImports #-}
module Acme.Api (serve) where

import qualified "acme-core" Acme.Core as Core
import Acme.Api.Routes

serve :: Core.Id -> Route
serve = route
