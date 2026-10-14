import { BlogPost } from "../../../pudu/generated";
import { AspectRatio, Card, CardContent, CardOverflow, Link, Stack, Typography } from "@mui/joy";
import { buildBlogPostUrl, formatBlogPostByline } from "../../../domain/news/blogPostPresentation";
import { openExternalUrl } from "../../../utils/navigation/openExternalUrl";

export default function SmallBlogPostPreview(props: BlogPost) {
    const { author, imageUrl, slug, createDateTime, summary, title } = props;
    const blogPostUrl = buildBlogPostUrl({ slug });
    const byline = formatBlogPostByline({ createDateTime, author });

    return (
        <Card
            variant="outlined"
            sx={{
                height: "100%",
                transition: "box-shadow 0.2s ease, transform 0.2s ease",
                "&:hover": {
                    boxShadow: "md",
                    transform: "translateY(-2px)",
                },
            }}
        >
            <CardOverflow>
                <AspectRatio ratio="16/9">
                    <img src={imageUrl!} loading="lazy" alt={title} />
                </AspectRatio>
            </CardOverflow>
            <CardContent>
                <Stack spacing={1}>
                    <Typography level="title-md">{title}</Typography>
                    <Typography level="body-xs" sx={{ color: "text.tertiary" }}>
                        {byline}
                    </Typography>
                    <Typography
                        level="body-sm"
                        sx={{
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            display: "-webkit-box",
                            WebkitLineClamp: 3,
                            WebkitBoxOrient: "vertical",
                        }}
                    >
                        {summary}
                    </Typography>
                    <Link
                        href={blogPostUrl}
                        onClick={(event) => void openExternalUrl(event, blogPostUrl)}
                        overlay
                        underline="none"
                        level="body-sm"
                        sx={{ fontWeight: "lg" }}
                    >
                        Read more
                    </Link>
                </Stack>
            </CardContent>
        </Card>
    );
}
