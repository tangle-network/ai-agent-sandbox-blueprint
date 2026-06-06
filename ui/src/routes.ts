import { flatRoutes } from '@react-router/fs-routes';

const routes: ReturnType<typeof flatRoutes> = flatRoutes({
  ignoredRouteFiles: ['**/*.test.*'],
});

export default routes;
